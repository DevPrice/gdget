# Manifest reference

`addons.toml` lists the addons a project uses. Commit it to your repository. It is both
the manifest and the lock file: each entry pins one exact archive by its sha256 hash,
and there are no version ranges.

`gdget add` writes entries for you. You can also edit the file by hand.

## Example

```toml
[addons.godot-slang]
version = "0.4.1"
url = "https://github.com/DevPrice/godot-slang/releases/download/v0.4.1/godot-slang.zip"
sha256 = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
path = "addons/godot-slang"
```

## Fields

Each `[addons.NAME]` table describes one addon. `NAME` is the install folder: the
addon installs to `addons/NAME/`. Use ASCII letters, digits, `-`, `_`, and `.`. If the
name contains a dot, quote it, as in `[addons."gut.v9"]`.

| Field | Required | Description |
|---|---|---|
| `url` | Yes | An `http://` or `https://` link to the addon's zip. |
| `sha256` | Yes | The sha256 hash of the zip. If a download doesn't match, gdget stops with an error. |
| `path` | No | The folder inside the zip to install, written with `/`. `.` means the root of the zip. If you omit it, gdget detects the folder. `gdget add` always sets it. |
| `version` | No | A label that `gdget status` displays. gdget doesn't use it to resolve anything. |

gdget rejects unknown fields so that typos cause an error instead of being ignored.

## How gdget finds the addon folder

Godot addons are usually packaged as `addons/NAME/` at the root of the zip. If `path`
isn't set, gdget uses the first of these rules that matches:

1.  `addons/NAME/`, at the root or inside a single top-level wrapper folder, such as the
    `repo-v1.2/` folder in GitHub source zips. gdget ignores `__MACOSX/` folders.
1.  The root, or the single wrapper folder, if it directly contains `plugin.cfg` or a
    `.gdextension` file.
1.  The only folder inside `addons/`, even if its name isn't `NAME`. gdget warns in this
    case, because `res://` paths inside the addon might expect the original name.

If no rule matches, or the zip contains more than one addon, gdget lists what it found
and stops. To choose a folder, set `path`, or pass `--path` to `gdget add`.
