//! Application layer: use cases orchestrating the domain over a repository port.
//!
//! `PasteService` depends on `Arc<dyn PasteRepository>`, so it is unit-tested
//! against an in-memory double and runs in production against SQLite unchanged.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rand::Rng;

use crate::domain::{
    BoxedError, Content, Paste, PasteId, PasteRepository, RepoError, ValidationError,
};

/// Length of an auto-generated paste id.
const GENERATED_ID_LEN: usize = 8;
/// Retry attempts on a (rare) id collision before giving up.
const MAX_GENERATION_ATTEMPTS: usize = 5;
/// Base62 alphabet for generated ids.
const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// Errors surfaced by the application layer.
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error(transparent)]
    Validation(ValidationError),
    #[error("not found")]
    NotFound,
    #[error("paste id already in use")]
    Conflict,
    #[error(transparent)]
    Backend(BoxedError),
}

impl From<RepoError> for ServiceError {
    fn from(err: RepoError) -> Self {
        match err {
            RepoError::Conflict => ServiceError::Conflict,
            RepoError::Backend(cause) => ServiceError::Backend(cause),
        }
    }
}

/// Use cases for creating and fetching pastes.
#[derive(Clone)]
pub struct PasteService {
    repo: Arc<dyn PasteRepository>,
}

impl PasteService {
    pub fn new(repo: Arc<dyn PasteRepository>) -> Self {
        Self { repo }
    }

    /// Create a paste from `content`, with optional syntax hint, TTL, and
    /// burn-after-read. The id is generated; a clash retries with a new id.
    pub async fn create(
        &self,
        content: String,
        syntax: Option<String>,
        ttl_seconds: Option<u64>,
        one_shot: bool,
    ) -> Result<Paste, ServiceError> {
        let content = Content::parse(content).map_err(ServiceError::Validation)?;
        let created_at = now_unix();
        let expires_at = ttl_seconds.map(|secs| created_at.saturating_add(secs as i64));

        for _ in 0..MAX_GENERATION_ATTEMPTS {
            let id = PasteId::from_trusted(generate_id(GENERATED_ID_LEN));
            let paste = Paste::new(
                id,
                content.clone(),
                syntax.clone(),
                created_at,
                expires_at,
                one_shot,
            );
            match self.repo.insert(&paste).await {
                Ok(()) => return Ok(paste),
                Err(RepoError::Conflict) => continue,
                Err(RepoError::Backend(cause)) => return Err(ServiceError::Backend(cause)),
            }
        }
        Err(ServiceError::Conflict)
    }

    /// Fetch a paste. Expired pastes are purged and reported as `NotFound`;
    /// a `one_shot` paste is deleted after this read (burn-after-read);
    /// otherwise the view counter is incremented. An invalid id cannot exist,
    /// so it maps to `NotFound`.
    pub async fn fetch(&self, id: String) -> Result<Paste, ServiceError> {
        let id = PasteId::parse(id).map_err(|_| ServiceError::NotFound)?;
        let paste = self.repo.get(&id).await?.ok_or(ServiceError::NotFound)?;

        if paste.is_expired(now_unix()) {
            let _ = self.repo.delete(&id).await; // best-effort lazy purge
            return Err(ServiceError::NotFound);
        }

        if paste.one_shot {
            let _ = self.repo.delete(&id).await; // burn-after-read
        } else {
            self.repo.increment_views(&id).await?;
        }
        Ok(paste)
    }

    /// Delete a paste, or `NotFound` if it does not exist.
    pub async fn delete(&self, id: String) -> Result<(), ServiceError> {
        let id = PasteId::parse(id).map_err(|_| ServiceError::NotFound)?;
        if self.repo.delete(&id).await? {
            Ok(())
        } else {
            Err(ServiceError::NotFound)
        }
    }

    /// Readiness check: confirm the backing store is reachable.
    pub async fn ready(&self) -> Result<(), ServiceError> {
        self.repo.ping().await?;
        Ok(())
    }
}

/// Current time as Unix seconds; clamps to 0 if the clock predates the epoch.
fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Generate a random base62 id of the given length.
fn generate_id(len: usize) -> String {
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::MAX_CONTENT_BYTES;
    use crate::infrastructure::InMemoryPasteRepository;

    fn service() -> PasteService {
        PasteService::new(Arc::new(InMemoryPasteRepository::default()))
    }

    #[test]
    fn generated_id_is_valid() {
        let id = generate_id(GENERATED_ID_LEN);
        assert_eq!(id.len(), GENERATED_ID_LEN);
        assert!(PasteId::parse(&id).is_ok());
    }

    #[tokio::test]
    async fn create_then_fetch_roundtrips() {
        let svc = service();
        let p = svc
            .create("hello world".to_owned(), Some("text".to_owned()), None, false)
            .await
            .unwrap();
        let fetched = svc.fetch(p.id.as_str().to_owned()).await.unwrap();
        assert_eq!(fetched.content.as_str(), "hello world");
        assert_eq!(fetched.syntax.as_deref(), Some("text"));
    }

    #[tokio::test]
    async fn create_rejects_empty_and_oversized_content() {
        let svc = service();
        assert!(matches!(
            svc.create(String::new(), None, None, false).await,
            Err(ServiceError::Validation(_))
        ));
        let big = "x".repeat(MAX_CONTENT_BYTES + 1);
        assert!(matches!(
            svc.create(big, None, None, false).await,
            Err(ServiceError::Validation(_))
        ));
    }

    #[tokio::test]
    async fn fetch_expired_is_not_found_and_purged() {
        let repo = Arc::new(InMemoryPasteRepository::default());
        let svc = PasteService::new(repo.clone());
        let expired = Paste::new(
            PasteId::parse("old").unwrap(),
            Content::parse("x").unwrap(),
            None,
            1_000,
            Some(1_001),
            false,
        );
        repo.insert(&expired).await.unwrap();

        assert!(matches!(
            svc.fetch("old".to_owned()).await,
            Err(ServiceError::NotFound)
        ));
        assert!(repo
            .get(&PasteId::parse("old").unwrap())
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn one_shot_paste_is_burned_after_first_fetch() {
        let svc = service();
        let p = svc
            .create("secret".to_owned(), None, None, true)
            .await
            .unwrap();
        let first = svc.fetch(p.id.as_str().to_owned()).await.unwrap();
        assert_eq!(first.content.as_str(), "secret");
        assert!(matches!(
            svc.fetch(p.id.as_str().to_owned()).await,
            Err(ServiceError::NotFound)
        ));
    }

    #[tokio::test]
    async fn non_one_shot_fetch_increments_views() {
        let repo = Arc::new(InMemoryPasteRepository::default());
        let svc = PasteService::new(repo.clone());
        let p = svc
            .create("body".to_owned(), None, None, false)
            .await
            .unwrap();
        svc.fetch(p.id.as_str().to_owned()).await.unwrap();
        let stored = repo.get(&p.id).await.unwrap().unwrap();
        assert_eq!(stored.views, 1);
    }

    #[tokio::test]
    async fn missing_id_is_not_found() {
        let svc = service();
        assert!(matches!(
            svc.fetch("missing".to_owned()).await,
            Err(ServiceError::NotFound)
        ));
        assert!(matches!(
            svc.delete("missing".to_owned()).await,
            Err(ServiceError::NotFound)
        ));
    }

    #[tokio::test]
    async fn delete_removes_paste() {
        let svc = service();
        let p = svc
            .create("x".to_owned(), None, None, false)
            .await
            .unwrap();
        svc.delete(p.id.as_str().to_owned()).await.unwrap();
        assert!(matches!(
            svc.fetch(p.id.as_str().to_owned()).await,
            Err(ServiceError::NotFound)
        ));
    }
}
