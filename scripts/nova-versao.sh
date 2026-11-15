#!/usr/bin/env bash
# Lança uma versão nova do Bingo Digital desktop:
#     bash scripts/nova-versao.sh 1.2.0
# 1. troca a versão em package.json, package-lock.json, src-tauri/Cargo.toml e Cargo.lock;
# 2. faz o commit "versão 1.2.0", cria a tag v1.2.0 e envia para o GitHub;
# 3. o GitHub Actions gera os instaladores e publica o release (~15 min);
# 4. o site puxa a versão sozinho (deploy, de hora em hora ou Admin › Versões › Buscar versão nova no GitHub).
set -euo pipefail
cd "$(dirname "$0")/.."

NOVA="${1:-}"
if ! [[ "$NOVA" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "Uso: bash scripts/nova-versao.sh 1.2.0" >&2
    exit 1
fi
ATUAL=$(node -p "require('./package.json').version")
if [ "$(printf '%s\n%s\n' "$ATUAL" "$NOVA" | sort -V | tail -1)" != "$NOVA" ] || [ "$ATUAL" = "$NOVA" ]; then
    echo "A versão nova ($NOVA) precisa ser maior que a atual ($ATUAL)." >&2
    exit 1
fi
if git rev-parse -q --verify "refs/tags/v$NOVA" >/dev/null; then
    echo "A tag v$NOVA já existe." >&2
    exit 1
fi
if [ -n "$(git status --porcelain)" ]; then
    echo "Há alterações sem commit. Faça o commit delas antes de lançar a versão:" >&2
    git status --short >&2
    exit 1
fi

echo "==> $ATUAL → $NOVA"
npm version "$NOVA" --no-git-tag-version >/dev/null
perl -0pi -e "s/^version = \"\Q$ATUAL\E\"/version = \"$NOVA\"/m" src-tauri/Cargo.toml
perl -0pi -e "s/(name = \"bingo-desktop\"\nversion = )\"\Q$ATUAL\E\"/\${1}\"$NOVA\"/" src-tauri/Cargo.lock
for f in package.json src-tauri/Cargo.toml src-tauri/Cargo.lock; do
    grep -q "\"$NOVA\"" "$f" || { echo "Não consegui trocar a versão em $f." >&2; git checkout -- package.json package-lock.json src-tauri/Cargo.toml src-tauri/Cargo.lock; exit 1; }
done

git add package.json package-lock.json src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "versão $NOVA"
git tag "v$NOVA"
git push origin HEAD
git push origin "v$NOVA"

echo
echo "Pronto: v$NOVA enviada. Acompanhe em https://github.com/pedroivoss/bingo-desktop/actions"
echo "Quando terminar (~15 min), o site publica sozinho (ou clique em Admin › Versões › Buscar versão nova no GitHub)."
