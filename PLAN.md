# gdget: plan

gdget (pronounced "gadget") installs pinned Godot 4 addons into a project from a committed
manifest. Its main users are the native GDExtension addons godot-slang and godot-verse,
whose prebuilt binaries must never be committed to game repos.

## Decisions

Settled in the interview (2026-09-25):

| Topic | Decision |
|---|---|
| Language | Rust (edition 2024). Uses crates, because the Rust standard library has no HTTP, zip or sha256 |
| Manifest | `addons.toml`, one `[addons.<name>]` table per addon. The source type comes from which key is present: `url` or `git` (see [Git sources](#git-sources)) |
| Override file | `addons.local.toml`, gitignored, with an `[overrides]` table |
| Locally modified addon | Skip it, warn, sync the others, then exit 1. `sync --force` overwrites |
| Commands | `sync [--force] [--check]`, `add <name> <url>` (re-pins if the name exists), `remove <name>`, `status` |
| `add` and `path` | `add` always writes the detected `path` to the manifest |
| Private assets | All public for now. `GITHUB_TOKEN` is sent only to github.com hosts. No Releases-API URL rewriting yet |
| Release targets | x86_64/aarch64 Windows (MSVC, static CRT), x86_64/aarch64 Linux (musl), x86_64/arm64 macOS |

`sync --locked` was dropped because `sync` never writes the manifest, so the flag would do
nothing. `sync --check` covers the CI need: it reports drift, changes nothing, and exits 1
if anything would change.

### Manifest

```toml
[addons.godot-slang]
version = "0.4.1"               # optional, display only
url = "https://github.com/…/godot-slang-0.4.1.zip"
sha256 = "9f86d08…"
path = "addons/godot-slang"     # optional; `add` always writes it
```

- An addon name must be a single safe path segment: no separators, `..`, or reserved Windows
  names.
- Unknown keys are rejected, which catches typos. The error says the key may need a newer
  gdget. This is how v2 keys such as `platforms` get added without designing them out.
- `add` and `remove` edit the file with `toml_edit`, so comments and ordering survive.

### Local overrides

```toml
[overrides]
godot-slang = "../godot-slang/build/addons/godot-slang"
```

Paths are relative to the project root. An override can point at either of two things, and
the same layout resolver that handles archives decides which:

- the addon folder itself, which is the recommended form, or
- a build-output root laid out like the release zip.

Overrides are linked into `addons/<name>`: a junction on Windows, a symlink elsewhere. No
marker is written, since that would write into the dev folder. The links gdget created are
recorded in `.gdget/state.toml`, so it only ever unlinks its own links and never deletes
anything through a link.

### Layout resolution

Godot's convention (Asset Library, GitHub zips) is `addons/<name>/…` at the archive root,
sometimes under one wrapper folder such as `repo-v1.2/`. An addon folder is recognized by a
`plugin.cfg` or a `*.gdextension` file. The resolver works the same way on a zip and on a
directory, and tries these rules in order:

1. Explicit `path`: use it, and fail if it is missing.
2. `addons/<name>/`, at the root or under a single wrapper folder.
3. The root itself is an addon folder.
4. A single top-level folder that is an addon folder.
5. Exactly one `addons/*/` folder with a different name: use it, but warn that `res://`
   paths inside it (such as the library paths in `.gdextension`) will probably break under
   the new name.
6. Otherwise, fail. The error lists the candidates it found and suggests setting `path`.

### Install mechanics

- **Project root:** the nearest ancestor directory that contains `addons.toml`. gdget warns
  if `project.godot` is not next to it.
- **Cache:** gdget checks these in order and uses the first one that applies:
  - `GDGET_CACHE_DIR`
  - `%LOCALAPPDATA%\gdget` on Windows
  - `$XDG_CACHE_HOME/gdget`
  - `~/Library/Caches/gdget` on macOS
  - `~/.cache/gdget`

  Archives are stored at `sha256/<hash>.zip` and are hashed while they stream to a temp
  file, then renamed into the cache. A cache hit is hashed again before use. A download
  whose hash doesn't match the pin is a hard error. A corrupt cache entry produces a
  warning, is deleted, and is downloaded again.
- **Two phases:** `sync` fetches and verifies every archive it needs before it touches
  `addons/`, so a network failure never leaves a partial state.
- **Extraction:** into `.gdget/staging/<name>-<rand>/`. `.gdget/` is on the same volume as
  `addons/`, so renames are atomic. It is a hidden directory with a `.gdignore`, so an open
  editor won't import from it. Entries that are absolute, contain `..` or are symlinks are
  rejected. Unix mode bits are kept.
- **Swap:**
  1. Rename `addons/<name>` to `.gdget/trash/`.
  2. Rename the staged folder into place.
  3. Delete the trash.

  Windows allows renaming a folder while a DLL inside it is loaded (tested), so a failed
  rename can't be the lock check. Before step 1, gdget tries to open every file in the old
  folder for writing, which a loaded image refuses with a sharing violation. It then
  leaves the old folder untouched, cleans up the staging folder, and tells the user to
  close the Godot editor. A sharing error on the step 1 rename gets the same message. If
  step 2 fails, the old folder is renamed back.
- **Marker:** `addons/<name>/.gdget.toml` holds the URL, sha256, resolved `path`, and a
  per-file hash list of what was extracted.
  - A no-op `sync` only compares the marker with the manifest. That is fast and does no
    hashing.
  - Files are hashed only when an addon has to be replaced, and for `status`. At that point
    added, removed or changed files count as modifications, except the `*.uid` and
    `*.import` files that Godot generates.
- **Ownership:** a folder with a marker, or a link recorded in `.gdget/state.toml`, belongs
  to gdget. Anything else in `addons/` is never touched. If an unowned folder is in the way
  of a manifest entry, that is an error.
- **Gitignore check:** after installing, gdget runs `git check-ignore -q addons/<name>/`. It
  warns if the folder is not ignored and skips the check quietly if git or a repo is
  missing. It also warns when `.gdget/` or `addons.local.toml` is not ignored.
- **HTTP:** `ureq` with `rustls`. `Authorization: Bearer $GITHUB_TOKEN` is sent only to
  `github.com`, `api.github.com` and `*.githubusercontent.com`, and is removed when a
  redirect goes to another host.
- **Output:** one plain line per action (`installed godot-slang 0.4.1`,
  `up to date godot-verse`). No spinners.
  - Color only on a TTY, and never when `NO_COLOR` is set.
  - With `GITHUB_ACTIONS=true`, warnings and errors are also printed as
    `::warning::`/`::error::` annotations.
  - Exit codes: 0 for success, 1 for any failure or for drift under `--check`, 2 for usage
    errors.

### Git sources

Settled in a second interview (2026-09-26):

| Topic | Decision |
|---|---|
| Fetching | The system `git`, not `gix` or host archive zips. Each remote gets a bare repo in the cache. `git archive --format=zip` feeds the existing extract, layout, and marker code |
| Manifest | `git`, `rev` (full commit hash, the pin), optional `ref` (what it was resolved from, for display and re-pinning). No `update` command yet |
| Multi-addon repos | One entry per folder, chosen with `--path`. NAME defaults to the path's last folder |
| CLI | `--git` and `--ref`. Without `--git`, URLs ending in `.git`, `ssh://`, and `USER@HOST:PATH` are git |
| Layout | For git sources with nothing under `addons/`, the root is the addon, even without `plugin.cfg` |
| Access | https and ssh with the user's git credentials, plus `GITHUB_TOKEN` for github.com |

- **Pinning trusts git's object hash.** That is SHA-1 with collision detection, unless the
  repository uses SHA-256. The marker still hashes every installed file with sha256 to
  detect local changes.
- **Submodules and Git LFS are out of scope**, because `git archive` omits them.

### Room for v2

- **Per-platform filtering:** the resolver produces a list of files, and a filter stage can
  go between resolving and extracting.

Not built yet.

## Crates

`clap` (derive), `serde`, `toml`, `toml_edit`, `ureq` (rustls), `zip`, `sha2`, `hex`,
`tempfile`, `junction` (Windows), `anyhow`/`thiserror`. For tests: `tiny_http` (the fixture
server on `127.0.0.1:0`) and `assert_cmd`. Fixture zips are built in the test code, so no
binary files are committed.

## Tasks

Each task is one or more commits and ends with `cargo test`, `cargo clippy -D warnings` and
`cargo fmt --check` passing.

1. **Scaffold.** `cargo init`, the clap command skeleton, the error and exit-code plumbing,
   the output/reporter module (TTY color, GitHub annotations), and a CI workflow that runs
   fmt, clippy and test on Windows, Linux and macOS.
2. **Manifest and override model.** Parse with `deny_unknown_fields`, validate names, look
   up the project root, and write with `toml_edit`. Unit tests for valid, invalid,
   unknown-key and bad-name input.
3. **Cache and fetcher.** Resolve the cache directory, download while hashing, keep cache
   writes atomic, and handle the GitHub token with cross-host header stripping. Build the
   `tiny_http` test harness here. Tests: download, cache hit, hash mismatch (both on
   download and for a corrupted cache file), and the token being sent only to allowed
   hosts.
4. **Layout resolver.** A tree abstraction over zip and directory, rules 1–6, and safe
   extraction that blocks zip-slip. Tests: each rule, the wrapper folder, the ambiguous
   layout error, and malicious entries.
5. **Marker and modification detection.** Per-file hashing and the `.uid`/`.import`
   exclusion. Tests: clean, modified, added and removed files.
6. **Installer.** Staging, the atomic swap, rollback, and the locked-file error. The Windows
   test holds a file open with `share_mode(0)` to simulate the loaded DLL, then checks that
   the old folder is intact and no staging folder is left behind.
7. **Linker.** Junctions and symlinks, and link records in `.gdget/state.toml`.
8. **`sync`.** Plan (diff the manifest and overrides against the disk), fetch, apply, remove
   what the manifest no longer lists, then run the gitignore checks. Add `--force` and
   `--check`.
9. **`status`.** For each addon: version/hash, installed / missing / outdated / modified /
   overridden, plus the unowned folders it ignores.
10. **`add` and `remove`.**
11. **Integration suite.** Runs the binary against the fixture server: fresh install, no-op
    re-sync, hash mismatch, manifest removal, override on then off, locally modified addon
    (with and without `--force`), ambiguous layout, unowned folder in the way, and
    `--check` exit codes.
12. **README.** Covers:
    - install
    - manifest format and the layout rules
    - local overrides
    - the `post-checkout`/`post-merge` hook snippet
    - a GitHub Actions snippet that downloads gdget and runs `gdget sync` before export
    - the addon-author notes below
13. **Release workflow.** Triggered by a tag. Cross-compiles the six targets (`cross` or
    native runners), packages each as `.zip` or `.tar.gz` with a sha256 file, and attaches
    them to the GitHub release.

Follow-up, as a separate task after task 10 works against a hand-uploaded zip: release
workflows for godot-slang and godot-verse that build every platform and attach
`<addon>-<version>.zip`, laid out as `addons/<name>/…`.

## Notes for addon authors (goes in the README)

- **Ship `.uid` files in the addon zip.** Since Godot 4.4, a script without one gets a fresh
  UID on every machine. Because `addons/` is gitignored, the game's scenes would then churn
  or break their `uid://` references to addon scripts.
- **Keep the folder name stable.** Paths in `.gdextension` are `res://addons/<name>/…`, so
  the manifest key must match the folder name the addon expects.
