# gdget

gdget ("gadget") installs pinned Godot 4 addons into a project from a committed
manifest. It is built for addons that ship prebuilt native libraries, such as
GDExtensions, which should never be committed to a game repository. The game repo
commits `addons.toml`; each checkout runs `gdget sync` to download, verify and install
exactly those versions.

- Every archive is pinned by sha256 and verified on every use.
- Downloads are cached per user, so switching branches or re-cloning costs nothing.
- Addons are installed as copies, so Godot's `.uid`/`.import` files never touch the
  cache.
- Installs are atomic: an addon is either fully replaced or left as it was.
- gdget only touches folders it installed, and never overwrites local edits without
  `--force`.
- It is a single static binary with no runtime to install.

## Install

Download the archive for your platform from the
[releases page](https://github.com/DevPrice/gdget/releases) and put `gdget` on your
`PATH`, or build it with Rust 1.88+:

```sh
cargo install --git https://github.com/DevPrice/gdget --locked
```

## Quick start

```sh
cd my-game                 # the folder with project.godot
gdget add godot-slang https://github.com/DevPrice/godot-slang/releases/download/v0.4.1/godot-slang.zip --version-label 0.4.1
git add addons.toml
```

Then add these lines to the game's `.gitignore`:

```gitignore
/addons/godot-slang/
/.gdget/
/addons.local.toml
```

Anyone who clones the repo runs `gdget sync`.

## Commands

| Command | What it does |
|---|---|
| `gdget sync` | Makes `addons/` match `addons.toml` plus local overrides. Installs, updates, links and removes as needed. |
| `gdget sync --check` | Reports what `sync` would change, without changing anything. Exits 1 on drift. |
| `gdget sync --force` | Like `sync`, but also overwrites or removes addons that were modified locally. |
| `gdget add <name> <url>` | Downloads the zip, pins its sha256 and folder in `addons.toml`, and installs it. Running it again on an existing name re-pins it, which is how you upgrade. Options: `--path <dir>` picks the folder inside the archive; `--version-label <v>` records a display-only version. |
| `gdget remove <name>` | Removes the entry from `addons.toml` and uninstalls the addon. |
| `gdget status` | Shows each addon's pin and state: installed, modified, outdated, missing, overridden, or not managed. |

Every command finds the project by walking up from the current directory (or from
`-C <dir>`) to the nearest folder that contains `addons.toml` or `project.godot`.

## `addons.toml`

This file is both the manifest and the lock file. There are no version ranges, only
exact pins.

```toml
[addons.godot-slang]
version = "0.4.1"                 # optional; shown by `status`, never used to resolve
url = "https://github.com/DevPrice/godot-slang/releases/download/v0.4.1/godot-slang.zip"
sha256 = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
path = "addons/godot-slang"       # folder inside the archive to install

[addons.godot-verse]
url = "https://github.com/DevPrice/godot-verse/releases/download/v1.2.0/godot-verse.zip"
sha256 = "…"
path = "."
```

- **Table key:** the install folder. `[addons.godot-slang]` installs to
  `addons/godot-slang/`. It must be a plain folder name made of ASCII letters, digits,
  `-`, `_` and `.`.
- **`url`:** an `http://` or `https://` link to a `.zip`.
- **`sha256`:** the hash of the zip. A download that doesn't match is a hard error.
- **`path`:** the folder inside the archive that becomes `addons/<name>/`, written with
  `/`. `.` means the archive root. `gdget add` always writes it. For a hand-written
  entry you can leave it out, and gdget will detect the folder using the rules below.

Unknown keys are errors, so typos surface right away. A newer gdget may add keys (such
as per-platform filtering), and an older gdget will then say it needs updating instead
of silently ignoring them.

### How gdget finds the addon folder in an archive

Godot's convention is `addons/<name>/…` at the archive root. gdget tries these rules in
order and uses the first match:

1. `path`, if set. It must exist in the archive.
2. `addons/<name>/`, either at the root or inside a single wrapper folder. GitHub's
   source zips put everything in a wrapper such as `repo-v1.2/`. A `__MACOSX/` folder is
   ignored.
3. The root itself, or the single wrapper folder, if it is an addon folder, meaning it
   directly contains `plugin.cfg` or a `*.gdextension` file.
4. The only folder under `addons/`, even if its name is different. gdget warns in this
   case, because `res://` paths inside the addon may expect the original name.

If none of these match, or several addons could match, gdget stops and lists what it
found. Set `path` (or pass `--path` to `add`) to choose.

## Local overrides: `addons.local.toml`

When you work on an addon, point the game at your local build instead of the pinned
zip. This file is personal and must be gitignored.

```toml
[overrides]
godot-slang = "../godot-slang/project/addons/godot-slang"
```

- Paths are relative to the project root.
- The path can be the addon folder itself, or a build-output folder laid out like the
  release zip. The same layout rules apply.
- On `sync`, gdget links `addons/<name>` to that folder. On Windows it uses a directory
  junction, which needs neither admin rights nor Developer Mode. Elsewhere it uses a
  symlink. Nothing is copied or downloaded, so rebuilding the addon updates the game
  immediately.
- To go back to the pinned version, delete the line (or the file) and run `gdget sync`.
  gdget removes only the link, never your build folder.
- An override can name an addon that isn't in `addons.toml` yet.

## What gdget owns

- **`addons/<name>/.gdget.toml`:** a marker in each installed folder. It records the
  source URL, the archive hash, and the hash of every installed file. `sync` uses it to
  tell what is up to date, what gdget installed, and what was edited locally.
- **Folders without a marker:** vendored or hand-made addons. gdget never touches them.
  If one sits where a manifest entry should install, that addon fails with an
  explanation.
- **Local edits:** an installed addon with local edits is not replaced or removed. `sync`
  warns, lists the changes, and exits 1 until you run `sync --force`. Godot's `.uid` and
  `.import` files, and the `~` copies Godot makes of libraries for hot reload, don't
  count as edits.
- **`.gdget/`:** gdget's working directory. It holds the staging area for atomic
  installs and `state.toml`, which records the override links gdget created. Keep it
  gitignored.

`sync` warns about any installed addon folder, `.gdget/` or `addons.local.toml` that git
would commit.

If the Godot editor has an addon's library loaded, Windows won't let that library be
replaced. gdget detects this before changing anything, leaves the addon as it was, and
asks you to close the editor.

## Git hooks

To sync automatically after switching branches or pulling, commit this script as both
`.githooks/post-checkout` and `.githooks/post-merge`:

```sh
#!/bin/sh
# post-checkout passes 0 as $3 for file checkouts; only sync on branch switches.
[ "$3" = "0" ] && exit 0
if ! command -v gdget >/dev/null 2>&1; then
  echo "gdget not found; skipping addon sync" >&2
  exit 0
fi
gdget sync || echo "gdget sync failed; fix the problem above and run it again" >&2
```

Then have each clone use the hooks:

```sh
git config core.hooksPath .githooks
```

On macOS and Linux, also run `chmod +x .githooks/*`.

Git ignores a hook's exit code for `post-checkout` and `post-merge`, so a failed sync
never blocks a checkout. It prints the problem instead.

## GitHub Actions

Run `gdget sync` before exporting the game:

```yaml
- name: Install gdget
  shell: bash
  run: |
    curl -fsSL https://github.com/DevPrice/gdget/releases/download/v0.1.0/gdget-x86_64-unknown-linux-musl.tar.gz \
      | tar -xz -C "$RUNNER_TEMP"
    echo "$RUNNER_TEMP" >> "$GITHUB_PATH"

- name: Cache addon archives
  uses: actions/cache@v4
  with:
    path: ~/.cache/gdget
    key: gdget-${{ hashFiles('addons.toml') }}
    restore-keys: gdget-

- name: Install Godot addons
  run: gdget sync
  env:
    GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

`GITHUB_TOKEN` is optional for public releases, but it raises GitHub's rate limits.
Under GitHub Actions, gdget's warnings and errors appear as workflow annotations. To
fail a job when the installed addons don't match the manifest, without changing them,
run `gdget sync --check`.

## Environment

| Variable | Effect |
|---|---|
| `GDGET_CACHE_DIR` | Where downloaded archives are cached. The default is `%LOCALAPPDATA%\gdget` on Windows, `$XDG_CACHE_HOME/gdget` if set, `~/Library/Caches/gdget` on macOS, and `~/.cache/gdget` otherwise. |
| `GITHUB_TOKEN` | Sent as a bearer token, but only over https to `github.com`, `api.github.com` and `*.githubusercontent.com`. It is never forwarded on redirects. |
| `ALL_PROXY`, `HTTPS_PROXY`, `HTTP_PROXY`, `NO_PROXY` | Standard proxy settings. |
| `NO_COLOR` | Disables colored output. Color is also off whenever output isn't a terminal. |

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success. |
| 1 | Any failure: a download or hash error, a blocked or locally modified addon, or drift under `sync --check`. |
| 2 | Invalid command-line usage. |

## Publishing an addon for gdget

- Zip the addon so it contains `addons/<name>/…` and attach the zip to a GitHub release.
  Keep `<name>` stable: paths in `.gdextension` files are `res://addons/<name>/…`, so
  games must install the addon under that same name.
- Include the `.uid` files Godot generates for your scripts and resources. Since Godot
  4.4, any file without one gets a fresh UID on every machine. Because game repos don't
  commit `addons/`, their scenes would then churn or break their `uid://` references to
  the addon.
- Don't put symbolic links in the zip. gdget refuses them, because they need Developer
  Mode on Windows and can point outside the addon. This includes the `Versions/Current`
  links inside macOS frameworks, so ship a flat `.framework` or a `.dylib`.

## License

[MIT](LICENSE)
