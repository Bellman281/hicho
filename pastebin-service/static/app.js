// Zero-knowledge pastebin client.
//
// All cryptography happens in the browser with AES-256-GCM. The AES key is
// derived with PBKDF2-SHA256 from a random key (placed in the URL fragment,
// never sent to the server) plus an optional password. The plaintext is a small
// JSON payload { t: text, s: syntax } so the language hint is encrypted too —
// the server only ever stores the opaque {v,iter,salt,iv,ct} envelope.
//
// Extras (all client-side, no server state): syntax highlighting, a locally
// generated QR code (key never leaves the page), i18n with browser-language
// detection, and a dark-theme toggle.

const PBKDF2_ITERATIONS = 100000;

// ---- i18n -------------------------------------------------------------------
const I18N = {
  en: {
    title: "Zero-Knowledge Pastebin",
    subtitle: "Encrypted in your browser with AES-256-GCM. The key lives in the link fragment and is never sent to the server.",
    contentPh: "Paste your text here…", syntaxLabel: "Language", synPlain: "Plain text",
    expiryLabel: "Expiry", ttlNever: "never", ttl1h: "1 hour", ttl1d: "1 day", ttl1w: "1 week",
    burnLabel: "burn after reading", passwordLabel: "Password (optional)",
    passwordPh: "leave empty for none", passwordHint: "If set, required in addition to the link — share it separately.",
    createBtn: "Encrypt & create", creating: "Encrypting…",
    resultLabel: "Share this link (keep the part after # secret):",
    pwNote: "Password-protected — share the password separately.",
    pwPrompt: "This paste is password-protected.", viewPasswordPh: "password", unlockBtn: "Unlock",
    decryptedNote: "Decrypted locally in your browser.", newPaste: "Create a new paste",
    errNotFound: "Paste not found, expired, or already viewed.",
    errDecrypt: "Wrong password (or corrupted link).",
    errCorrupt: "This paste is corrupted or in an unknown format.", failed: "Failed: ",
  },
  es: {
    title: "Pastebin de conocimiento cero",
    subtitle: "Cifrado en tu navegador con AES-256-GCM. La clave vive en el fragmento del enlace y nunca se envía al servidor.",
    contentPh: "Pega tu texto aquí…", syntaxLabel: "Lenguaje", synPlain: "Texto sin formato",
    expiryLabel: "Caducidad", ttlNever: "nunca", ttl1h: "1 hora", ttl1d: "1 día", ttl1w: "1 semana",
    burnLabel: "borrar tras leer", passwordLabel: "Contraseña (opcional)",
    passwordPh: "dejar vacío para ninguna", passwordHint: "Si se define, se requiere además del enlace — compártela por separado.",
    createBtn: "Cifrar y crear", creating: "Cifrando…",
    resultLabel: "Comparte este enlace (mantén en secreto lo que va tras #):",
    pwNote: "Protegido con contraseña — compártela por separado.",
    pwPrompt: "Este paste está protegido con contraseña.", viewPasswordPh: "contraseña", unlockBtn: "Desbloquear",
    decryptedNote: "Descifrado localmente en tu navegador.", newPaste: "Crear un nuevo paste",
    errNotFound: "Paste no encontrado, caducado o ya visto.",
    errDecrypt: "Contraseña incorrecta (o enlace dañado).",
    errCorrupt: "Este paste está dañado o en un formato desconocido.", failed: "Error: ",
  },
  fr: {
    title: "Pastebin à divulgation nulle",
    subtitle: "Chiffré dans votre navigateur avec AES-256-GCM. La clé se trouve dans le fragment du lien et n'est jamais envoyée au serveur.",
    contentPh: "Collez votre texte ici…", syntaxLabel: "Langage", synPlain: "Texte brut",
    expiryLabel: "Expiration", ttlNever: "jamais", ttl1h: "1 heure", ttl1d: "1 jour", ttl1w: "1 semaine",
    burnLabel: "détruire après lecture", passwordLabel: "Mot de passe (optionnel)",
    passwordPh: "laisser vide pour aucun", passwordHint: "Si défini, requis en plus du lien — partagez-le séparément.",
    createBtn: "Chiffrer et créer", creating: "Chiffrement…",
    resultLabel: "Partagez ce lien (gardez secret ce qui suit #):",
    pwNote: "Protégé par mot de passe — partagez-le séparément.",
    pwPrompt: "Ce paste est protégé par mot de passe.", viewPasswordPh: "mot de passe", unlockBtn: "Déverrouiller",
    decryptedNote: "Déchiffré localement dans votre navigateur.", newPaste: "Créer un nouveau paste",
    errNotFound: "Paste introuvable, expiré ou déjà consulté.",
    errDecrypt: "Mot de passe incorrect (ou lien corrompu).",
    errCorrupt: "Ce paste est corrompu ou dans un format inconnu.", failed: "Échec : ",
  },
  de: {
    title: "Zero-Knowledge-Pastebin",
    subtitle: "Im Browser mit AES-256-GCM verschlüsselt. Der Schlüssel steht im Link-Fragment und wird nie an den Server gesendet.",
    contentPh: "Text hier einfügen…", syntaxLabel: "Sprache", synPlain: "Klartext",
    expiryLabel: "Ablauf", ttlNever: "nie", ttl1h: "1 Stunde", ttl1d: "1 Tag", ttl1w: "1 Woche",
    burnLabel: "nach dem Lesen löschen", passwordLabel: "Passwort (optional)",
    passwordPh: "leer lassen für keins", passwordHint: "Wenn gesetzt, zusätzlich zum Link erforderlich — separat teilen.",
    createBtn: "Verschlüsseln & erstellen", creating: "Verschlüsseln…",
    resultLabel: "Diesen Link teilen (Teil nach # geheim halten):",
    pwNote: "Passwortgeschützt — Passwort separat teilen.",
    pwPrompt: "Dieses Paste ist passwortgeschützt.", viewPasswordPh: "Passwort", unlockBtn: "Entsperren",
    decryptedNote: "Lokal in deinem Browser entschlüsselt.", newPaste: "Neues Paste erstellen",
    errNotFound: "Paste nicht gefunden, abgelaufen oder bereits angesehen.",
    errDecrypt: "Falsches Passwort (oder beschädigter Link).",
    errCorrupt: "Dieses Paste ist beschädigt oder in unbekanntem Format.", failed: "Fehler: ",
  },
};
const LANG_NAMES = { en: "English", es: "Español", fr: "Français", de: "Deutsch" };

let lang = "en";
const t = (key) => (I18N[lang] && I18N[lang][key]) || I18N.en[key] || key;

function pickLang() {
  let saved = null;
  try { saved = localStorage.getItem("lang"); } catch (e) {}
  if (saved && I18N[saved]) return saved;
  const nav = (navigator.language || "en").slice(0, 2).toLowerCase();
  return I18N[nav] ? nav : "en";
}
function applyI18n() {
  document.documentElement.lang = lang;
  document.title = t("title");
  document.querySelectorAll("[data-i18n]").forEach((el) => { el.textContent = t(el.dataset.i18n); });
  document.querySelectorAll("[data-i18n-ph]").forEach((el) => { el.placeholder = t(el.dataset.i18nPh); });
}

// ---- base64 helpers ---------------------------------------------------------
function bytesToB64(bytes) {
  let bin = "";
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
  return btoa(bin);
}
function b64ToBytes(b64) {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}
const b64UrlEncode = (b64) => b64.replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
function b64UrlDecode(s) { s = s.replace(/-/g, "+").replace(/_/g, "/"); while (s.length % 4) s += "="; return s; }

// ---- crypto -----------------------------------------------------------------
async function deriveKey(urlKey, password, salt, iter) {
  const pw = new TextEncoder().encode(password || "");
  const material = new Uint8Array(urlKey.length + pw.length);
  material.set(urlKey, 0);
  material.set(pw, urlKey.length);
  const base = await crypto.subtle.importKey("raw", material, "PBKDF2", false, ["deriveKey"]);
  return crypto.subtle.deriveKey(
    { name: "PBKDF2", salt, iterations: iter, hash: "SHA-256" },
    base, { name: "AES-GCM", length: 256 }, false, ["encrypt", "decrypt"],
  );
}
async function encryptPayload(text, syntax, password) {
  const plaintext = JSON.stringify({ t: text, s: syntax || "" });
  const urlKey = crypto.getRandomValues(new Uint8Array(32));
  const salt = crypto.getRandomValues(new Uint8Array(16));
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const key = await deriveKey(urlKey, password, salt, PBKDF2_ITERATIONS);
  const ct = new Uint8Array(await crypto.subtle.encrypt({ name: "AES-GCM", iv }, key, new TextEncoder().encode(plaintext)));
  const envelope = JSON.stringify({ v: 2, iter: PBKDF2_ITERATIONS, salt: bytesToB64(salt), iv: bytesToB64(iv), ct: bytesToB64(ct) });
  return { envelope, keyStr: b64UrlEncode(bytesToB64(urlKey)) };
}
async function decryptPayload(env, keyStr, password) {
  const key = await deriveKey(b64ToBytes(b64UrlDecode(keyStr)), password, b64ToBytes(env.salt), env.iter || PBKDF2_ITERATIONS);
  const ptBuf = await crypto.subtle.decrypt({ name: "AES-GCM", iv: b64ToBytes(env.iv) }, key, b64ToBytes(env.ct));
  const text = new TextDecoder().decode(ptBuf);
  try { const o = JSON.parse(text); return { t: typeof o.t === "string" ? o.t : text, s: o.s || "" }; }
  catch (e) { return { t: text, s: "" }; } // tolerate non-JSON payloads
}

// ---- syntax highlighting (lightweight, generic, XSS-safe) -------------------
const escapeHtml = (s) => s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" }[c]));
const HL = /(\/\*[\s\S]*?\*\/|\/\/[^\n]*|#[^\n]*)|("(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|`(?:\\.|[^`\\])*`)|(\b\d[\w.]*\b)|(\b(?:if|else|elif|for|while|do|return|fn|function|def|const|let|var|class|struct|enum|impl|trait|pub|use|mod|import|from|export|match|case|switch|break|continue|new|public|private|protected|static|void|int|float|double|char|bool|boolean|string|str|true|false|null|nil|None|True|False|async|await|try|catch|finally|throw|package|interface|extends|implements|select|insert|update|delete|where|echo)\b)/g;
function highlight(text, syntax) {
  const esc = escapeHtml(text);
  if (!syntax || esc.length > 200000) return null; // plain text or too big: caller uses textContent
  return esc.replace(HL, (m, c, s, n, k) =>
    c ? `<span class="tok-comment">${c}</span>` :
    s ? `<span class="tok-string">${s}</span>` :
    n ? `<span class="tok-number">${n}</span>` :
        `<span class="tok-keyword">${k}</span>`);
}

// ---- API --------------------------------------------------------------------
async function createPaste(content, ttlSeconds, oneShot) {
  const body = { content, one_shot: oneShot };
  if (ttlSeconds) body.ttl_seconds = ttlSeconds;
  const res = await fetch("/api/pastes", {
    method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error("HTTP " + res.status);
  return res.json();
}
async function fetchRaw(id) {
  const res = await fetch("/raw/" + encodeURIComponent(id));
  if (res.status === 404) return null;
  if (!res.ok) throw new Error("HTTP " + res.status);
  return res.text();
}

// ---- UI ---------------------------------------------------------------------
const $ = (id) => document.getElementById(id);

function renderPayload(payload) {
  const html = highlight(payload.t, payload.s);
  if (html === null) $("output").textContent = payload.t;
  else $("output").innerHTML = html;
}

async function onCreate() {
  const content = $("content").value;
  if (!content) return;
  const btn = $("createBtn");
  btn.disabled = true;
  btn.textContent = t("creating");
  try {
    const password = $("password").value;
    const { envelope, keyStr } = await encryptPayload(content, $("syntax").value, password);
    const created = await createPaste(envelope, $("ttl").value ? Number($("ttl").value) : null, $("oneshot").checked);
    const link = `${location.origin}/#${created.id}.${keyStr}`;
    $("link").innerHTML = `<a href="${link}">${link}</a>`;
    try {
      const qr = qrcode(0, "M");
      qr.addData(link);
      qr.make();
      $("qr").innerHTML = qr.createImgTag(4, 8, "QR code for the share link");
    } catch (e) { $("qr").innerHTML = ""; }
    $("result").classList.remove("hidden");
    $("pwNote").classList.toggle("hidden", !password);
  } catch (err) {
    alert(t("failed") + err.message);
  } finally {
    btn.disabled = false;
    btn.textContent = t("createBtn");
  }
}

async function showView(id, keyStr) {
  $("create").classList.add("hidden");
  $("view").classList.remove("hidden");

  const blob = await fetchRaw(id);
  if (blob === null) { $("error").textContent = t("errNotFound"); return; }
  let env;
  try { env = JSON.parse(blob); } catch (e) { $("error").textContent = t("errCorrupt"); return; }

  try {
    renderPayload(await decryptPayload(env, keyStr, ""));
    return;
  } catch (e) { $("pwPrompt").classList.remove("hidden"); }

  $("unlockBtn").addEventListener("click", async () => {
    try {
      renderPayload(await decryptPayload(env, keyStr, $("viewPassword").value));
      $("pwPrompt").classList.add("hidden");
      $("error").textContent = "";
    } catch (e) { $("error").textContent = t("errDecrypt"); }
  });
}

function initChrome() {
  // language selector
  const sel = $("lang");
  for (const code of Object.keys(I18N)) {
    const opt = document.createElement("option");
    opt.value = code;
    opt.textContent = LANG_NAMES[code] || code;
    sel.appendChild(opt);
  }
  sel.value = lang;
  sel.addEventListener("change", () => {
    lang = sel.value;
    try { localStorage.setItem("lang", lang); } catch (e) {}
    applyI18n();
  });
  // theme toggle
  $("themeBtn").addEventListener("click", () => {
    const next = document.documentElement.getAttribute("data-theme") === "dark" ? "light" : "dark";
    document.documentElement.setAttribute("data-theme", next);
    try { localStorage.setItem("theme", next); } catch (e) {}
  });
}

function init() {
  lang = pickLang();
  initChrome();
  applyI18n();

  const hash = location.hash.slice(1);
  const dot = hash.indexOf(".");
  if (dot > 0) {
    showView(hash.slice(0, dot), hash.slice(dot + 1));
  } else {
    $("createBtn").addEventListener("click", onCreate);
  }
}
init();
