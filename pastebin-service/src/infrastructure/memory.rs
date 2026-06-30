//! In-memory [`PasteRepository`] backed by a `Mutex<HashMap>`.
//!
//! The lock is held only for a synchronous map operation — never across an
//! `.await` — so the futures stay `Send` and the runtime never blocks. Entries
//! are removed on delete, so memory tracks live pastes exactly (no leak).

use std::collections::HashMap;
use std::sync::Mutex;

use crate::domain::{Paste, PasteId, PasteRepository, RepoError};

/// Thread-safe in-memory paste store.
#[derive(Debug, Default)]
pub struct InMemoryPasteRepository {
    pastes: Mutex<HashMap<String, Paste>>,
}

impl InMemoryPasteRepository {
    /// Acquire the lock, recovering data even if a previous holder panicked.
    fn guard(&self) -> std::sync::MutexGuard<'_, HashMap<String, Paste>> {
        self.pastes.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait::async_trait]
impl PasteRepository for InMemoryPasteRepository {
    async fn insert(&self, paste: &Paste) -> Result<(), RepoError> {
        let mut pastes = self.guard();
        if pastes.contains_key(paste.id.as_str()) {
            return Err(RepoError::Conflict);
        }
        pastes.insert(paste.id.as_str().to_owned(), paste.clone());
        Ok(())
    }

    async fn get(&self, id: &PasteId) -> Result<Option<Paste>, RepoError> {
        Ok(self.guard().get(id.as_str()).cloned())
    }

    async fn increment_views(&self, id: &PasteId) -> Result<bool, RepoError> {
        match self.guard().get_mut(id.as_str()) {
            Some(paste) => {
                paste.views += 1;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    async fn delete(&self, id: &PasteId) -> Result<bool, RepoError> {
        Ok(self.guard().remove(id.as_str()).is_some())
    }

    async fn ping(&self) -> Result<(), RepoError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Content;

    fn sample() -> Paste {
        Paste::new(
            PasteId::parse("abc").unwrap(),
            Content::parse("body").unwrap(),
            None,
            1_700_000_000,
            None,
            false,
        )
    }

    #[tokio::test]
    async fn insert_conflicts_then_views_and_delete_report_presence() {
        let repo = InMemoryPasteRepository::default();
        let id = PasteId::parse("abc").unwrap();

        assert!(!repo.increment_views(&id).await.unwrap());
        repo.insert(&sample()).await.unwrap();
        assert!(matches!(repo.insert(&sample()).await, Err(RepoError::Conflict)));

        assert!(repo.increment_views(&id).await.unwrap());
        assert_eq!(repo.get(&id).await.unwrap().unwrap().views, 1);

        assert!(repo.delete(&id).await.unwrap());
        assert!(!repo.delete(&id).await.unwrap());
    }
}
