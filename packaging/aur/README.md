# Arch Linux / AUR

The `cibitube-bin` recipe packages the published Linux x86_64 release. It installs
Cibitube, its desktop entry, a 512×512 icon and the MIT license. All downloaded
files are pinned to a release tag and verified with SHA-256.

The current recipe targets **v0.2.10**. Changes made after that tag, including the
package-manager update integration, require a new upstream binary release before
they can be included in this binary package.

From `packaging/aur/cibitube-bin`, build and install with:

```sh
makepkg -si
```

Installation through `paru` becomes available after the recipe is published to
AUR:

```sh
paru -S cibitube-bin
```

Only `PKGBUILD`, `.SRCINFO` and `cibitube.desktop` belong in the AUR Git repository.
Publication requires an AUR account with SSH access. Package archives and the
`src/` and `pkg/` directories stay outside that repository.

When updating the recipe, set `pkgver` to the published version, reset `pkgrel`
for a new upstream version, verify the downloaded files and update their hashes.
Regenerate and review metadata before publishing:

```sh
makepkg --printsrcinfo > .SRCINFO
makepkg --verifysource
```

The package installs `usr/share/cibitube/package-manager` with the value `pacman`.
Cibitube versions supporting this marker show package-manager update instructions
and keep self-update from replacing the installed executable. Standalone copies
retain their built-in updater.
