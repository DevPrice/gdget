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

A project without `addons.toml` can still sync if it has local overrides.

## gdget add

Downloads an addon, pins its hash or commit and its folder in `addons.toml`, and
installs it.

```sh
gdget add [NAME] SOURCE [--git] [--ref REF] [--path PATH] [--version-label LABEL]
```

Replace the following:

- `NAME`: optional. The install folder name. The addon installs to `addons/NAME/`. Use
  ASCII letters, digits, `-`, `_`, and `.`. If you omit it, gdget uses the first of
  these that applies:
  - the last folder of `PATH`
  - the name of the source's only folder under `addons/`
  - the asset name, for an Asset Store asset
  - the repository name, for a git repository
- `SOURCE`: where to get the addon. It's one of the following:
  - An `https://` link to the addon's zip. gdget warns if you use `http://`.
  - An asset from the [Godot Asset Store](https://store.godotengine.org), written
    `PUBLISHER/ASSET@VERSION`, as in the store page's address
    `store.godotengine.org/asset/PUBLISHER/ASSET/`. If you omit `@VERSION`, gdget uses
    the latest stable release. The leading `v` in a version is optional, so `@6.0.0`
    matches `v6.0.0`.
  - A git repository: an `https://` URL ending in `.git`, an `ssh://` URL, or an address
    such as `git@github.com:DevPrice/godot-addons.git`. For a repository URL without
    `.git`, add `--git`. Installing from a repository needs `git` on your `PATH`.
- `REF`: optional, for a git repository. The branch, tag, or full commit hash to
  install. If you omit it, gdget uses the repository's default branch. gdget pins the
  commit that `REF` points at now and records `REF` next to it.
- `PATH`: optional. The folder inside the archive or repository to install, if gdget
  can't detect it. For details, see
  [How gdget finds the addon folder](manifest.md#how-gdget-finds-the-addon-folder).
- `LABEL`: optional. A version label that `status` displays. gdget doesn't use it to
  resolve anything. For an Asset Store asset, it defaults to the release's version.

For example, the following command installs version 6.0.0 of
[godot-slang](https://store.godotengine.org/asset/devprice/godot-slang/):

```sh
gdget add devprice/godot-slang@v6.0.0
```

gdget pins an Asset Store asset by the release's download link on the store, so `sync`
doesn't depend on the store's API.

To install one addon from a repository that holds several, as folders at its root,
pass the folder as `PATH`. The addon is named after the folder:

```sh
gdget add https://github.com/DevPrice/godot-addons.git --path inventory
```

To upgrade an addon, run `gdget add` again with the new URL, version, or `REF`. To
upgrade an Asset Store asset to its latest stable release, omit the version. To move a
git addon to the latest commit of its branch, run `gdget add` again with the same
`REF`.

## gdget remove

Removes an addon from `addons.toml` and uninstalls it.

```sh
gdget remove NAME
```

If the addon has local changes, gdget removes the entry but keeps the folder, and exits
with status 1. To delete the folder anyway, run `gdget sync --force`.

## gdget link

Uses a local folder in place of an addon's pinned release. gdget records the override in
`addons.local.toml` and links `addons/NAME/` to the folder. For details, see
[Develop an addon locally](local-development.md).

```sh
gdget link [NAME] PATH
```

Replace the following:

- `NAME`: optional. The install folder name. If you omit it, gdget uses the name of the
  folder's only folder under `addons/`, or the folder's own name if it's an addon folder.
- `PATH`: the local build: the addon folder itself, or a folder laid out like the release
  zip. If `PATH` is at most two folders above the project folder, as a sibling checkout
  is, gdget records it as a relative path so it keeps working if you move both.
  Otherwise, gdget records the absolute path.

To point an override at a different folder, run `gdget link` again.

## gdget unlink

Removes an addon's override from `addons.local.toml` and reinstalls its pinned release.
If the addon isn't in `addons.toml`, gdget removes the link.

```sh
gdget unlink NAME
```

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
| `GDGET_STORE_URL` | The Asset Store that `add` looks up assets in. Defaults to `https://store.godotengine.org`. |
| `GITHUB_TOKEN` | A token sent to GitHub to raise rate limits and to fetch private repositories. gdget sends it only over HTTPS to `github.com`, `api.github.com`, and `*.githubusercontent.com`, and never forwards it on redirects. In GitHub Actions, the default token can read only the workflow's own repository, so fetching another private repository needs a personal access token. |
| `GIT_ALLOW_PROTOCOL` | The transports git may use to fetch repositories. If it isn't set, gdget allows only `https` and `ssh`. |
| `ALL_PROXY`, `HTTPS_PROXY`, `HTTP_PROXY`, `NO_PROXY` | Standard proxy settings. |
| `NO_COLOR` | Turns off colored output. Output is also uncolored when it isn't a terminal. |

## Exit status

| Status | Meaning |
|---|---|
| 0 | The command succeeded. |
| 1 | The command failed, an addon was blocked or has local changes, or `sync --check` found differences. |
| 2 | The command line is invalid. |
