# Releasing HUSH

Distribution is automated. Pushing to the **`release` branch** cuts a release on
every channel; pushing to `main` only runs CI (fmt / clippy / build / tests).

## Cut a release

1. Bump `version` in the root `Cargo.toml` (`[workspace.package]`) — it must be
   **strictly greater** than the last `v*` tag, or the release job fails.
2. `git push origin main:release` (or merge/push `main` into `release`).

The `Release` workflow then:
- tags `v<version>` and creates a GitHub Release with the built
  `hush_v<version>_x86_64-linux.tar.gz` (+ `.sha256`),
- builds + GPG-signs the Flatpak and publishes the repo to GitHub Pages,
- pushes updated `PKGBUILD` + `.SRCINFO` to the `hush-mic-bin` and `hush-mic` AUR repos
  (the plain `hush` names are taken by an unrelated Lua shell).

Nix users get the new version from `github:UMCEKO/hush` on their next update.

## One-time setup

### Secrets (repo → Settings → Secrets and variables → Actions)
| kind | name | value |
|------|------|-------|
| secret | `AUR_SSH_PRIVATE_KEY` | private key of your AUR account (public key added at aur.archlinux.org → My Account) |
| secret | `FLATPAK_GPG_PRIVATE_KEY` | from `scripts/gen-flatpak-key.sh` |
| variable | `FLATPAK_GPG_FINGERPRINT` | from `scripts/gen-flatpak-key.sh` |

### Flatpak signing key
Run `bash scripts/gen-flatpak-key.sh` once: it writes `packaging/flatpak/hush.gpg`
+ `hush.flatpakrepo` (commit both) and prints the secret + fingerprint to set above.

### GitHub Pages
Settings → Pages → Source = **GitHub Actions**.

### AUR first publication
The CI job pushes updates; the packages must exist first. Once, from a machine with
your AUR SSH key:
```sh
for p in hush-mic-bin hush-mic; do
  git clone ssh://aur@aur.archlinux.org/$p.git
  cp packaging/aur/$p/{PKGBUILD,.SRCINFO} $p/    # set pkgver to the first release
  (cd $p && git add -A && git commit -m init && git push)
done
```

## Install (users)

- **AUR:** `paru -S hush-mic-bin` (prebuilt) or `paru -S hush-mic` (from source).
- **Flatpak:** `flatpak remote-add --if-not-exists hush https://umceko.github.io/hush/index.flatpakrepo && flatpak install hush io.github.umceko.hush`
- **Nix:** `nix profile install github:UMCEKO/hush`, or the home-manager module
  (`services.hush.enable = true`).

On first launch HUSH downloads the NVIDIA Maxine runtime (~1.2 GB) + the per-GPU
model from `cdn.hush.umceko.com` — nothing NVIDIA ships in any package.
