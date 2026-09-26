# Publish an addon for gdget

This page is for addon authors. It describes how to package a release that gdget can
install.

## Package the release

1.  Put the addon in a zip file at `addons/NAME/`, where `NAME` is the folder name your
    addon expects.

    Keep `NAME` stable across releases. Paths in `.gdextension` files use
    `res://addons/NAME/`, so games must install the addon under that exact name.

2.  Include the `.uid` files that Godot generates for your scripts and resources.

    Starting with Godot 4.4, a file without a `.uid` file gets a different UID on each
    machine. Games don't commit `addons/`, so their scenes would lose or change their
    `uid://` references to your addon.

3.  Don't include symbolic links. gdget rejects them because they need Developer Mode on
    Windows and can point outside the addon. This includes the `Versions/Current` links
    in macOS frameworks, so ship a flat `.framework` or a `.dylib` instead.

4.  Use file names that are valid on Windows: no trailing dots or spaces, none of the
    characters `<>"|?*`, and no reserved names such as `CON` or `NUL`. gdget rejects
    these names on every platform so that an addon installs the same way everywhere.

5.  Publish the zip as a release on the [Godot Asset Store](https://store.godotengine.org),
    attach it to a GitHub release, or both.

    Put only one folder under `addons/`. gdget then installs your addon under the right
    name without the user passing one.

## Share an addon from a git repository

An addon written in GDScript doesn't need a release build, so users can install it
straight from its repository. gdget pins a commit and installs one folder from it. Lay
out the repository in one of these ways:

- The addon at `addons/NAME/`, the same layout as a release zip. The rest of the
  repository, such as a demo project, isn't installed.
- The addon's files at the root of the repository. Users install the root under the
  repository's name, so name the repository what the addon expects to be called.
- Several addons, each in a folder at the root. Users choose one with `--path`, and it
  installs under the folder's name.

The rules for release zips apply here too: commit the `.uid` files, keep folder names
stable, and don't commit symbolic links. Tags give users a readable `--ref` to pin.

To install a folder with `gdget add`, the repository must be reachable over `https://`
or SSH.

## Test the release

To check that gdget can install the zip, run one of these commands in a test project:

```sh
gdget add PUBLISHER/ASSET@VERSION
gdget add URL
gdget add REPOSITORY --ref TAG
```

Replace the following:

- `PUBLISHER/ASSET@VERSION`: your asset on the Asset Store, as in
  `devprice/godot-slang@v6.0.0`.
- `URL`: the zip's download link from the release page.
- `REPOSITORY`: your repository's clone URL, as in
  `https://github.com/DevPrice/godot-addons.git`. Add `--path FOLDER` if it holds more
  than one addon.
- `TAG`: the tag or branch to install.
