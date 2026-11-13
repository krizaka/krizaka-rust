# Changelog

Products pin a tag (`tag = "vX.Y.Z"`). Before 1.0 a minor version may break the API.

## 0.4.0 — 2026-11-13

Minor, not patch: the API breaks in four places since 0.3.0 (krizaka-internal#2959, krizaka-restricted#321).

### Breaking
- **A resolved path is a `ScopedPath`.** `ScopeStore::resolve(_with_home)`, `Access::resolve` and `Kit::resolve` return it
  (was `PathBuf`): `as_path()`, `into_path_buf()`, `AsRef<Path>`, `open()`. `files::list_dir` and `files::preview` take
  `&ScopedPath`; a preview reads one descriptor, checked still inside the grant.
- **Typed errors.** No function answers `Result<_, String>`. Each module has its own enum with a `code()`; a command fails
  with `CommandError`, which the page receives as `{ "code", "message" }` (was a string). `Transport::post_form` returns
  `TransportError`; `device_flow::Failure::Other(String)` is replaced by `Transport`, `Provider` and `Malformed`.
- **Limits are passed by the app.** `files::list_dir` and `files::preview` take `&Limits`; the `MAX_*` constants are
  replaced by `DEFAULT_*` and `Limits::send()`; the `list_folder` command accepts an optional `limits` object.
  `Entry.kind` and `Preview.kind` are `Kind` (JSON unchanged).
- A secret's key or value is no longer echoed in an error; `update::require_pubkey` refuses an updater without a public key.

### Changed
- More MIME types are reported (one table, `files::TYPES`); `.heic` is `image/heic`.
- Listings are sorted, then cut.
- Film: the shared join counts screens, `finish` reports what it skipped; the frame list is read without `-safe 0`.
