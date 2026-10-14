#!/usr/bin/env bash
# Installe gabier comme service systemd.
# A lancer en root depuis la racine du projet:
#   sudo ./install.sh [chemin/vers/le/binaire]
set -euo pipefail

BIN_SRC="${1:-target/release/gabier}"
PREFIX="/usr/local/bin"
CONF_DIR="/etc/gabier"
UNIT="/etc/systemd/system/gabier.service"
SERVICE_USER="gabier"

die() {
  echo "erreur: $*" >&2
  exit 1
}

[[ "${EUID}" -eq 0 ]] || die "a lancer en root (sudo ./install.sh)"
[[ -f "${BIN_SRC}" ]] || die "binaire introuvable: ${BIN_SRC} (lance d'abord: cargo build --release)"
[[ -f "systemd/gabier.service" ]] || die "a lancer depuis la racine du projet"
command -v systemctl >/dev/null || die "systemd introuvable sur cette machine"

# Compte de service sans shell ni home.
if ! id -u "${SERVICE_USER}" >/dev/null 2>&1; then
  NOLOGIN="$(command -v nologin || echo /sbin/nologin)"
  useradd --system --no-create-home --shell "${NOLOGIN}" "${SERVICE_USER}"
  echo "utilisateur ${SERVICE_USER} cree"
fi

install -m 0755 "${BIN_SRC}" "${PREFIX}/gabier"
echo "binaire installe dans ${PREFIX}/gabier"

install -d -m 0750 -o root -g "${SERVICE_USER}" "${CONF_DIR}"

FRESH_CONFIG=0
if [[ ! -f "${CONF_DIR}/config.toml" ]]; then
  install -m 0640 -o root -g "${SERVICE_USER}" config.example.toml "${CONF_DIR}/config.toml"
  FRESH_CONFIG=1
  echo "configuration installee dans ${CONF_DIR}/config.toml"
else
  echo "configuration existante conservee: ${CONF_DIR}/config.toml"
fi

install -m 0644 systemd/gabier.service "${UNIT}"
systemctl daemon-reload
systemctl enable gabier >/dev/null
echo "service active au demarrage"

echo
if [[ "${FRESH_CONFIG}" -eq 1 ]]; then
  cat <<'MSG'
Il reste trois choses a faire:

  1. Renseigner le webhook Discord:
       sudo nano /etc/gabier/config.toml

  2. Verifier que tout repond:
       sudo -u gabier gabier check --config /etc/gabier/config.toml
       sudo -u gabier gabier test  --config /etc/gabier/config.toml

  3. Demarrer:
       sudo systemctl start gabier
       sudo journalctl -u gabier -f
MSG
else
  cat <<'MSG'
Redemarre le service pour prendre en compte la nouvelle version:

  sudo systemctl restart gabier
  sudo journalctl -u gabier -f
MSG
fi
