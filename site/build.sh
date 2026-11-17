#!/usr/bin/env bash
# Builds the static home (FR at /, EN at /en/) into the directory given as $1 (default: public-site).
# The version comes from Cargo.toml and the test link from the source: nothing of the sort is typed in the templates.
set -euo pipefail
cd "$(dirname "$0")/.."
out="${1:-public-site}"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
[[ -n "$version" ]] || { echo "no version in Cargo.toml" >&2; exit 1; }
sha="${GITHUB_SHA:-$(git rev-parse HEAD)}"
line="$(grep -n 'fn refuses_paths_outside_the_scope_and_dotdot_escapes' src/scope.rs | head -1 | cut -d: -f1)"
[[ -n "$line" ]] || { echo "the scope test moved: update site/build.sh" >&2; exit 1; }
repo="https://github.com/krizaka/krizaka-rust"
test_url="$repo/blob/$sha/src/scope.rs#L$line"
if [[ -n "${GITHUB_RUN_ID:-}" ]]; then run_url="$repo/actions/runs/$GITHUB_RUN_ID"; else run_url="$repo/actions/workflows/pages.yml"; fi

render() { # lang path otherpath otherlang othername, then KEY=VALUE pairs on stdin
  local dest="$1"; shift
  local page; page="$(cat site/page.tpl.html)"
  while IFS='=' read -r k v; do
    [[ -n "$k" ]] || continue
    page="${page//@$k@/$v}"
  done
  mkdir -p "$(dirname "$dest")"
  printf '%s\n' "$page" > "$dest"
  if grep -q '@[A-Z_]*@' "$dest"; then echo "unfilled placeholder in $dest" >&2; grep -o '@[A-Z_]*@' "$dest" | sort -u >&2; exit 1; fi
}

common="VERSION=$version
TESTURL=$test_url
RUNURL=$run_url"

render "$out/index.html" <<EOT
$common
LANG=fr
PATH=/
OTHERPATH=/en/
OTHERLANG=en
OTHERNAME=English
NAVLABEL=Navigation
TITLE=Krizaka Rust : le socle commun des apps Mac de Krizaka (en développement)
DESC=Documentation de l'API de krizaka-rust et tauri-plugin-krizaka : dossiers autorisés, fichiers, garde de navigation, liens profonds, menu, secrets et canal de mise à jour.
LEDE=Le socle commun des apps Mac de Krizaka (en développement) : Tauri 2, Apache-2.0. Version courante : $version.
H_DOCS=Documentation de l'API
CARD1=Les règles : Rust pur, sans dépendance à Tauri, testées sur n'importe quelle machine.
CARD2=La colle qui applique les règles : commandes, événements, Trousseau, menu natif.
H_USE=Pour l'utiliser
USE=Pas encore sur crates.io : on dépend du dépôt, à un tag.
PROOF=Ce que ça garantit, par un exemple : un chemin hors du dossier autorisé est refusé (<code>..</code> compris).
C_GRANT=un dossier autorisé par l'utilisateur
C_ESCAPE=hors du dossier : refusé, jamais lu
TESTLINK=Le test dont il est tiré (src/scope.rs)
H_SRC=Source
REPO=Dépôt sur GitHub
CHANGELOG=Journal des changements
H_FAMILY=Les autres bibliothèques ouvertes de Krizaka
D_UI=le système de design, pour le web et React Native.
D_TS=les briques TypeScript côté serveur.
D_JAVA=le POM parent et les briques Spring Boot.
D_LAB=des expériences gardées pour plus tard.
BUILT=Construit par <code>cargo doc</code> à chaque main verte :
RUNLINK=voir la construction
EOT

render "$out/en/index.html" <<EOT
$common
LANG=en
PATH=/en/
OTHERPATH=/
OTHERLANG=fr
OTHERNAME=Français
NAVLABEL=Navigation
TITLE=Krizaka Rust: the shared base of Krizaka's Mac apps (in development)
DESC=API documentation of krizaka-rust and tauri-plugin-krizaka: folder grants, scoped files, navigation guard, deep links, menu, secrets and the update channel.
LEDE=The shared base of Krizaka's Mac apps (in development): Tauri 2, Apache-2.0. Current version: $version.
H_DOCS=API documentation
CARD1=The rules: pure Rust, no Tauri dependency, tested on any machine.
CARD2=The glue that applies the rules: commands, events, the Keychain, the native menu.
H_USE=Use it
USE=Not on crates.io yet: depend on the repository, at a tag.
PROOF=What it guarantees, by example: a path outside the granted folder is refused (<code>..</code> included).
C_GRANT=a folder the user granted
C_ESCAPE=outside the folder: refused, never read
TESTLINK=The test it comes from (src/scope.rs)
H_SRC=Source
REPO=Repository on GitHub
CHANGELOG=Changelog
H_FAMILY=Krizaka's other open-source libraries
D_UI=the design system, for web and React Native.
D_TS=server-side TypeScript building blocks.
D_JAVA=the parent POM and the Spring Boot building blocks.
D_LAB=experiments kept for later.
BUILT=Built from <code>cargo doc</code> on every green <code>main</code>:
RUNLINK=see the build
EOT
cp site/CNAME "$out/CNAME"
