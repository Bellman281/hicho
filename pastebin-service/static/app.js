// Zero-knowledge pastebin client.
//
// All cryptography happens in the browser with AES-256-GCM (WebCrypto). The
// actual encryption key is derived with PBKDF2-SHA256 from TWO inputs:
//   1) a random key placed in the URL fragment (never sent to the server), and
//   2) an optional user password (never stored or sent anywhere).
// So the link alone decrypts a passwordless paste, but a password-protected
// paste needs BOTH the link and the password. The server only ever stores the
// opaque envelope below and has zero knowledge of the plaintext or the key.
//
// Link format:   <origin>/#<paste-id>.<url-key>
// Stored blob:   JSON { v, iter, salt, iv, ct }  (all but the key are public)

const PBKDF2_ITERATIONS = 100000;

// ---- base64 helpers (chunked, so large pastes don't blow the call stack) ----
function bytesToB64(bytes) {
  let binary = "";
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    binary += String.fromCharCode.apply(null, bytes.subarray(i, i + CHUNK));
  }
  return btoa(binary);
}
function b64ToBytes(b64) {
  const binary = atob(b64);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
  return out;
}
function b64UrlEncode(b64) {
  return b64.replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}
function b64UrlDecode(s) {
  s = s.replace(/-/g, "+").replace(/_/g, "/");
  while (s.length % 4) s += "=";
  return s;
}

// ---- key derivation ----
// Derive an AES-GCM key from (random url-key bytes || password) via PBKDF2.
// With an empty password this still stretches the url-key with the salt, so the
// stored format is identical whether or not a password is used.
async function deriveKey(urlKeyBytes, password, salt, iterations) {
  const pwBytes = new TextEncoder().encode(password || "");
  const material = new Uint8Array(urlKeyBytes.length + pwBytes.length);
  material.set(urlKeyBytes, 0);
  material.set(pwBytes, urlKeyBytes.length);
  const baseKey = await crypto.subtle.importKey("raw", material, "PBKDF2", false, ["deriveKey"]);
  return crypto.subtle.deriveKey(
    { name: "PBKDF2", salt, iterations, hash: "SHA-256" },
    baseKey,
    { name: "AES-GCM", length: 256 },
    false,
    ["encrypt", "decrypt"],
  );
}

// ---- crypto ----
async function encryptText(plaintext, password) {
  const urlKey = crypto.getRandomValues(new Uint8Array(32)); // 256-bit, goes in the URL fragment
  const salt = crypto.getRandomValues(new Uint8Array(16));
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const key = await deriveKey(urlKey, password, salt, PBKDF2_ITERATIONS);
  const data = new TextEncoder().encode(plaintext);
  const ctBuf = await crypto.subtle.encrypt({ name: "AES-GCM", iv }, key, data);
  const envelope = JSON.stringify({
    v: 2,
    iter: PBKDF2_ITERATIONS,
    salt: bytesToB64(salt),
    iv: bytesToB64(iv),
    ct: bytesToB64(new Uint8Array(ctBuf)),
  });
  const keyStr = b64UrlEncode(bytesToB64(urlKey));
  return { envelope, keyStr };
}

// Decrypt an envelope with the url-key + (optional) password. Throws on a wrong
// password / wrong key / tampering (AES-GCM is authenticated).
async function decryptEnvelope(env, keyStr, password) {
  const salt = b64ToBytes(env.salt);
  const iv = b64ToBytes(env.iv);
  const ct = b64ToBytes(env.ct);
  const urlKey = b64ToBytes(b64UrlDecode(keyStr));
  const key = await deriveKey(urlKey, password, salt, env.iter || PBKDF2_ITERATIONS);
  const ptBuf = await crypto.subtle.decrypt({ name: "AES-GCM", iv }, key, ct);
  return new TextDecoder().decode(ptBuf);
}

// ---- API ----
async function createPaste(content, ttlSeconds, oneShot) {
  const body = { content, one_shot: oneShot };
  if (ttlSeconds) body.ttl_seconds = ttlSeconds;
  const res = await fetch("/api/pastes", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error("create failed: HTTP " + res.status);
  return res.json(); // { id, ... }
}
async function fetchRaw(id) {
  const res = await fetch("/raw/" + encodeURIComponent(id));
  if (res.status === 404) return null; // not found, expired, or already burned
  if (!res.ok) throw new Error("fetch failed: HTTP " + res.status);
  return res.text();
}

// ---- UI ----
const $ = (id) => document.getElementById(id);

async function onCreate() {
  const content = $("content").value;
  if (!content) return;
  const btn = $("createBtn");
  btn.disabled = true;
  btn.textContent = "Encrypting…";
  try {
    const password = $("password").value;
    const { envelope, keyStr } = await encryptText(content, password);
    const created = await createPaste(envelope, $("ttl").value ? Number($("ttl").value) : null, $("oneshot").checked);
    const link = `${location.origin}/#${created.id}.${keyStr}`;
    $("link").innerHTML = `<a href="${link}">${link}</a>`;
    $("result").classList.remove("hidden");
    if (password) $("pwNote").classList.remove("hidden");
  } catch (err) {
    alert("Failed: " + err.message);
  } finally {
    btn.disabled = false;
    btn.textContent = "Encrypt & create";
  }
}

async function showView(id, keyStr) {
  $("create").classList.add("hidden");
  $("view").classList.remove("hidden");

  const blob = await fetchRaw(id);
  if (blob === null) {
    $("error").textContent = "Paste not found, expired, or already viewed.";
    return;
  }
  let env;
  try {
    env = JSON.parse(blob);
  } catch (_e) {
    $("error").textContent = "This paste is corrupted or in an unknown format.";
    return;
  }

  // Try without a password first; if that fails, it's password-protected.
  try {
    $("output").textContent = await decryptEnvelope(env, keyStr, "");
    return;
  } catch (_e) {
    $("pwPrompt").classList.remove("hidden");
  }

  $("unlockBtn").addEventListener("click", async () => {
    try {
      $("output").textContent = await decryptEnvelope(env, keyStr, $("viewPassword").value);
      $("pwPrompt").classList.add("hidden");
      $("error").textContent = "";
    } catch (_e) {
      $("error").textContent = "Wrong password (or corrupted link).";
    }
  });
}

function init() {
  const hash = location.hash.slice(1);
  const dot = hash.indexOf(".");
  if (dot > 0) {
    showView(hash.slice(0, dot), hash.slice(dot + 1));
  } else {
    $("createBtn").addEventListener("click", onCreate);
  }
}
init();
