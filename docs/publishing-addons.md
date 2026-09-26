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

5.  Attach the zip to a GitHub release.

## Test the release

To check that gdget can install the zip, run this command in a test project:

```sh
gdget add NAME URL
```

Replace `URL` with the zip's download link from the release page.
