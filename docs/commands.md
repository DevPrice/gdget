# Command reference

Every command finds the project by searching upward from the current directory for the
nearest folder that contains `addons.toml` or `project.godot`. To run a command in
another folder, pass `-C DIRECTORY`.

## gdget sync

Makes `addons/` match `addons.toml` and your [local overrides](local-development.md).
It installs missing addons, updates outdated ones, links overrides, and removes addons
it installed that are no longer in the manifest.

```sh
gdget sync [--force | --check]
```

| Flag | Description |
|---|---|
| `--check` | Reports what `sync` would change without changing anything. Exits with status 1 if anything is out of date. |
| `--force` | Overwrites or removes installed addons even if they have local changes. |

`sync` downloads and verifies every archive it needs before it changes `addons/`. If a
download fails or a hash doesn't match, `sync` stops without changing anything.

## gdget add

Downloads an addon, pins its hash and folder in `addons.toml`, and installs it.

```sh
gdget add NAME URL [--path PATH] [--version-label LABEL]
```

Replace the following:

- `NAME`: the install folder name. The addon installs to `addons/NAME/`. Use ASCII
  letters, digits, `-`, `_`, and `.`.
- `URL`: an `https://` link to the addon's zip. gdget warns if you use `http://`.
- `PATH`: optional. The folder inside the archive to install, if gdget can't detect it.
  For details, see [How gdget finds the addon folder](manifest.md#how-gdget-finds-the-addon-folder).
- `LABEL`: optional. A version label that `status` displays. gdget doesn't use it to
  resolve anything.

To upgrade an addon, run `gdget add` again with the same name and the new URL.

## gdget remove

Removes an addon from `addons.toml` and uninstalls it.

```sh
gdget remove NAME
```

If the addon has local changes, gdget removes the entry but keeps the folder, and exits
with status 1. To delete the folder anyway, run `gdget sync --force`.

## gdget status

Lists each addon with its pinned version and hash, and its state: installed, modified,
outdated, missing, overridden, or not managed by gdget. `status` doesn't change
anything, and exits with status 0 unless it can't read the project's files.

```sh
gdget status
```

## Which folders gdget changes

gdget changes only the folders it installed or linked:

- **Installed addons** contain a `.gdget.toml` marker that records the source, the
  archive hash, and the hash of every installed file. gdget uses the marker to detect
  outdated addons and local changes.
- **Folders without a marker**, such as vendored or hand-made addons, are never changed.
  If one is where a manifest entry should install, that addon fails with an error.
- **Local changes** block an update or removal until you run `sync --force`. Files that
  Godot generates, such as `.uid` and `.import` files and the `~` copies of libraries it
  makes for hot reload, don't count as changes.
- **`.gdget/`** is gdget's working folder. It holds a lock file, the staging area for
  atomic installs, and a record of the override links gdget created.

On Windows, if the Godot editor has an addon's library loaded, gdget leaves the addon
unchanged and asks you to close the editor.

After a sync, gdget warns about any addon folder, `.gdget/`, or `addons.local.toml` that
Git would commit.

## Environment variables

| Variable | Description |
|---|---|
| `GDGET_CACHE_DIR` | The download cache folder. Defaults to `%LOCALAPPDATA%\gdget` on Windows, `$XDG_CACHE_HOME/gdget` if that variable is set, `~/Library/Caches/gdget` on macOS, and `~/.cache/gdget` otherwise. |
| `GITHUB_TOKEN` | A token sent to GitHub to raise rate limits. gdget sends it only over HTTPS to `github.com`, `api.github.com`, and `*.githubusercontent.com`, and never forwards it on redirects. |
| `ALL_PROXY`, `HTTPS_PROXY`, `HTTP_PROXY`, `NO_PROXY` | Standard proxy settings. |
| `NO_COLOR` | Turns off colored output. Output is also uncolored when it isn't a terminal. |

## Exit status

| Status | Meaning |
|---|---|
| 0 | The command succeeded. |
| 1 | The command failed, an addon was blocked or has local changes, or `sync --check` found differences. |
| 2 | The command line is invalid. |
