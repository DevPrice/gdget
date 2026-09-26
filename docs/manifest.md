# Manifest reference

`addons.toml` lists the addons a project uses. Commit it to your repository. It is both
the manifest and the lock file: each entry pins one exact zip by its sha256 hash, or
one exact git commit, and there are no version ranges.

`gdget add` writes entries for you. You can also edit the file by hand.

## Example

```toml
[addons.godot-slang]
version = "0.4.1"
url = "https://github.com/DevPrice/godot-slang/releases/download/v0.4.1/godot-slang.zip"
sha256 = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
path = "addons/godot-slang"

[addons.inventory]
git = "https://github.com/DevPrice/godot-addons.git"
ref = "master"
rev = "86a47feb2e0e8f2b675302cefa30e58536f6625f"
path = "inventory"
```

## Fields

Each `[addons.NAME]` table describes one addon. `NAME` is the install folder: the
addon installs to `addons/NAME/`. Use ASCII letters, digits, `-`, `_`, and `.`. If the
name contains a dot, quote it, as in `[addons."gut.v9"]`.

An entry gets the addon either from a zip, with `url` and `sha256`, or from a git
repository, with `git` and `rev`. It can't have both.

| Field | Required | Description |
|---|---|---|
| `url` | For a zip | An `http://` or `https://` link to the addon's zip. |
| `sha256` | For a zip | The sha256 hash of the zip. If a download doesn't match, gdget stops with an error. |
| `git` | For a repository | The repository's address: an `https://` or `ssh://` URL, or `USER@HOST:PATH`, as in `git@github.com:DevPrice/godot-addons.git`. |
| `rev` | For a repository | The full hash of the commit to install. `sync` installs this commit, whatever `ref` currently points at. |
| `ref` | No | The branch or tag that `rev` came from. `gdget status` displays it. To move the pin, run `gdget add` again. |
| `path` | No | The folder inside the zip or repository to install, written with `/`. `.` means the root. If you omit it, gdget detects the folder. `gdget add` always sets it. |
| `version` | No | A label that `gdget status` displays. gdget doesn't use it to resolve anything. |

gdget rejects unknown fields so that typos cause an error instead of being ignored.

Installing from a repository needs `git` on your `PATH`. gdget fetches only the commit
it needs and uses your usual git credentials, so private repositories work if
`git clone` does. It installs the commit's files as `git archive` exports them, which
leaves out submodules and doesn't download Git LFS files.

## How gdget finds the addon folder

Godot addons are usually packaged as `addons/NAME/` at the root of the zip or
repository. If `path` isn't set, gdget uses the first of these rules that matches:

1.  `addons/NAME/`, at the root or inside a single top-level wrapper folder, such as the
    `repo-v1.2/` folder in GitHub source zips. gdget ignores `__MACOSX/` folders.
1.  The root, or the single wrapper folder, if it directly contains `plugin.cfg` or a
    `.gdextension` file.
1.  The only folder inside `addons/`, even if its name isn't `NAME`. gdget warns in this
    case, because `res://` paths inside the addon might expect the original name.
1.  For a git repository with nothing in `addons/`, the root of the repository. Addons
    written only in GDScript often have no `plugin.cfg`, so this rule lets a repository
    that is one addon install without `path`.

If no rule matches, or the source contains more than one addon, gdget lists what it
found and stops. To choose a folder, set `path`, or pass `--path` to `gdget add`.

A repository with several addon folders at its root matches the last rule, so gdget
installs the whole repository as one addon. Set `path` to install one of the folders.
