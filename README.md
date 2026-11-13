# Bingo Digital — app desktop

App para Windows, macOS e Linux (Tauri 2 + Rust + TypeScript). Conversa com o servidor Laravel
pela API descrita em `../Bingo/docs/API-DESKTOP.md`.

- **Demonstração** sem conta: 40 cartelas geradas no computador, funciona sem internet.
- **Com a conta**: baixa as festas e cartelas da licença, faz o sorteio até offline (mesmas regras
  da web) e envia tudo ao sincronizar. A licença é assinada pelo servidor (Ed25519) e vale 10 dias
  sem internet; depois disso, ou com o relógio do computador voltado para trás, o app apaga os
  dados sincronizados (os PDFs salvos pelo cliente ficam).

## Estrutura

| Pasta | O quê |
|---|---|
| `src/main.ts` | Telas (entrada, festas, sorteio, telão). |
| `src-tauri/src/licenca.rs` | Confere a licença com a chave pública embutida. |
| `src-tauri/src/jogo.rs` | Regras do sorteio (espelham o `SorteioService` do servidor) e embaralhamento. |
| `src-tauri/src/db.rs` | SQLite local (pasta de dados do app). |
| `src-tauri/src/lib.rs` | Comandos: login, sincronizar, ações do sorteio, conferir, ranking, demonstração. |
| `public/vendor` | Bootstrap, tema, ícones e SweetAlert2 copiados do site (funcionam offline). |

## Desenvolver

Requisitos: Node 20+, Rust (`rustup`), no Linux as libs do WebKitGTK.

```bash
npm install
npm run tauri dev                     # abre o app apontando para o servidor de dev
cd src-tauri && cargo test --lib      # regras do jogo, embaralhamento igual ao PHP, licença
cargo test --lib -- --ignored         # ponta a ponta contra o servidor de dev (cliente@teste.local)
```

Servidor e chave: em desenvolvimento usa `http://ivocompany.local:8000/src/Bingo/public` e a chave
pública do `.env` de dev. O build de produção recebe `BINGO_API_URL` e `BINGO_CHAVE_PUBLICA` como
variáveis de ambiente **na compilação**.

## Gerar os instaladores (grátis, GitHub Actions)

1. Suba este projeto para um repositório no GitHub.
2. Em **Settings → Secrets and variables → Actions → Variables**, crie `BINGO_API_URL` e
   `BINGO_CHAVE_PUBLICA` (a chave pública do servidor de produção: `php artisan bingo:desktop-chaves`),
   `BINGO_FIXAR_CERTIFICADO=letsencrypt` (opcional) e, em **Secrets**, `TAURI_SIGNING_PRIVATE_KEY`.
3. Suba a versão: `git tag v1.0.0 && git push origin v1.0.0`.
4. O workflow `.github/workflows/release.yml` gera Windows (.exe/.msi), macOS (Apple Silicon e
   Intel, .dmg) e Linux (.deb/.rpm/.AppImage) e cria um **rascunho de Release** com os arquivos.
5. Publique o Release no GitHub (os arquivos precisam ser baixáveis sem login: repositório público, ou
   copie os arquivos para um lugar público) e cole as impressões digitais do resumo em `BINGO_DESKTOP_HASHES`.
6. No site, **Admin → Computadores → Versões do app**: cole o `latest.json` do release, preencha os links
   dos instaladores (aparecem na página inicial) e clique em **Publicar**. Pronto: cada computador recebe a
   versão nova na próxima vez que abrir o app ou sincronizar. Marque **Obrigatória** quando for uma correção
   importante: app mais antigo para de sincronizar até atualizar.

## Como os computadores se atualizam

- O app pergunta ao **servidor do Bingo Digital** (não ao GitHub):
  `GET /api/desktop/v1/atualizacao/{target}/{arch}/{versão atual}` → `204` (nada) ou o JSON do updater do Tauri.
- Procura ao abrir e **depois de cada sincronização**. Baixa em segundo plano e pergunta se pode reiniciar;
  "Depois" = instala sozinho quando o app for fechado (qualquer forma de fechar). Com o telão aberto não
  aparece nada: o aviso espera o operador sair do telão.
- Versão obrigatória (ou servidor respondendo `426 atualizar_app`): tela de atualização sem "depois".
- O pacote é conferido com a assinatura da chave `~/.tauri/bingo-digital.key`: arquivo trocado no caminho é recusado.
- Em produção o endereço precisa ser `https` (o updater recusa `http`).

Repositório privado: o plano gratuito do GitHub tem 2.000 minutos/mês (minutos de macOS contam 10×);
um release completo usa cerca de 300–400. Repositório público não tem limite.

## Proteções (12D)

| Proteção | Como funciona |
|---|---|
| Código das telas ofuscado | `vite build` minifica e ofusca (`rollup-plugin-obfuscator`); sem DevTools, menu "Inspecionar" e atalhos na versão de produção. |
| Regras e segredos fora do app | Cartelas nascem no servidor; licença assinada (Ed25519, só a chave pública no app); todo sorteio é conferido de novo no servidor. |
| Impressão digital do app | O app manda o SHA-256 do próprio executável (`X-App-Hash`). Admin → **Computadores** marca app que não é o oficial ou que mudou sem mudar a versão. O workflow de release mostra as impressões oficiais: cole em `BINGO_DESKTOP_HASHES` no `.env` do site. |
| Rastreio de vazamento | Cada página de PDF leva `ref {festa}-{computador}-{1ª cartela}` no canto. Admin → Computadores → "Rastrear PDF" diz de qual conta/computador saiu. |
| Fixação de certificado | Com `BINGO_FIXAR_CERTIFICADO=letsencrypt` no build, o app só confia nas raízes do Let's Encrypt (`src-tauri/certs`): proxies que interceptam o tráfego deixam de funcionar. Só ligue se o certificado do servidor for Let's Encrypt (o CloudPanel usa). |
| Atualização automática assinada | `tauri-plugin-updater`: o app procura versão nova ao abrir e ao sincronizar e só instala pacotes assinados com a sua chave (`~/.tauri/bingo-digital.key`, grátis). Quem decide a versão oferecida é o painel (Admin → Versões do app). |

**Guarde a chave `~/.tauri/bingo-digital.key`** (e coloque em Secrets → `TAURI_SIGNING_PRIVATE_KEY`). Sem ela não dá
para publicar atualizações para quem já instalou. Build local de produção:
`TAURI_SIGNING_PRIVATE_KEY=~/.tauri/bingo-digital.key npm run tauri build`.

> Nenhuma proteção de programa instalado é 100%. O que impede prejuízo é o servidor: crack não cria
> cartela válida, não forja licença e não muda o resultado aceito na nuvem. O resto dificulta e denuncia.

## Sem certificado pago (decisão: custo zero)

O app sai **sem assinatura paga**. Na primeira instalação os sistemas avisam:

- **Windows**: "O Windows protegeu o computador" → **Mais informações** → **Executar assim mesmo**.
- **macOS**: abra o app; se aparecer que não pode ser aberto, vá em **Ajustes do Sistema →
  Privacidade e Segurança** → **Abrir Mesmo Assim** (uma vez só). O app tem assinatura local
  (`signingIdentity: "-"`), então o macOS não diz que está "danificado".
- **Linux**: `.deb`/`.rpm` instalam normalmente; o `.AppImage` precisa de permissão de execução.

Se um dia valer a pena: certificado Apple (US$ 99/ano) e certificado de assinatura do Windows removem
os avisos — o workflow já tem o lugar para isso (ver docs do Tauri, "Sign").


## Sincronização, imagens e PDFs no computador

- Ao abrir o app (e logo depois de entrar na conta) ele sincroniza **todas** as festas da licença: envia o que
  foi feito sem internet e baixa festa, cartelas, imagens e patrocinadores. Depois procura atualização.
- Sem internet, segue com o que já está no computador até a licença vencer (10 dias desde o último contato);
  passou disso, os dados sincronizados são apagados e voltam inteiros ao entrar de novo na conta.
- As imagens ficam em **Documentos › Bingo Digital › <festa>**: `Imagens/` (cabeçalho, logo, marca d'água,
  coringa) e `Patrocinadores/` (`01 - Nome.png`...), espelhando o painel. Os PDFs são sugeridos em `PDFs/`.
- Sincronização, geração de PDFs e atualização esmaecem e travam o app até terminar, com barra de progresso.
- O telão mostra o carrossel dos patrocinadores (dá para desligar no painel do sorteio).
