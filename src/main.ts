// Bingo Digital — app desktop (tela). As regras e os dados ficam no Rust (src-tauri/src);
// aqui só desenhamos e chamamos os comandos.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { openUrl, revealItemInDir } from '@tauri-apps/plugin-opener';
import { open as escolherPasta } from '@tauri-apps/plugin-dialog';
import qrcode from 'qrcode-generator';

declare const Swal: any;
declare const confetti: any;

type Json = any;
const app = document.getElementById('app')!;
const LETRAS = ['B', 'I', 'N', 'G', 'O'];
const letra = (n: number) => LETRAS[Math.floor((n - 1) / 15)] ?? '';
const esc = (t: unknown) => String(t ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]!));
const SAIR_PARA_LOGIN = ['sem_login', 'token_invalido', 'dispositivo_revogado', 'expirada', 'bloqueada', 'relogio', 'licenca_invalida', 'conta_desativada'];

let situacao: Json = null;
let festaId: number | null = null;
let rodadaId: number | null = null;
let estado: Json = null;

/** Erro do Rust: texto simples ou JSON {codigo, message}. */
function lerErro(e: unknown): { codigo: string; message: string; dispositivos?: Json[] } {
  try { const j = JSON.parse(String(e)); if (j && j.message) return j; } catch { /* texto simples */ }
  return { codigo: 'erro', message: String(e) };
}

async function chamar<T = Json>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    const err = lerErro(e);
    if (SAIR_PARA_LOGIN.includes(err.codigo)) {
      await iniciar(err.message);
      throw new Error('__tratado__');
    }
    throw err;
  }
}

const toast = (msg: string, icon = 'warning') =>
  Swal.fire({ toast: true, position: 'top-end', timer: 2600, showConfirmButton: false, icon, title: msg });

const mostrarErro = (e: Json) => { if (e?.message !== '__tratado__') toast(e?.message ?? String(e), 'error'); };


const fmtData = (iso?: string) => (iso ? new Date(iso).toLocaleString('pt-BR', { dateStyle: 'short', timeStyle: 'short' }) : '—');

function topo(direita = ''): string {
  return `<header class="app-topo so-operador"><div class="container-fluid py-2 d-flex align-items-center justify-content-between gap-2">
    <div class="app-marca"><span class="bola">B</span> Bingo Digital <span class="badge text-bg-light border fw-normal">desktop ${esc(situacao?.versao)}</span></div>
    <div class="d-flex align-items-center gap-2">${direita}
      <button class="btn btn-sm btn-outline-secondary" id="btn-tema" aria-label="Alternar tema claro/escuro"><i class="fas fa-circle-half-stroke"></i></button>
    </div></div></header>`;
}

function ligarTema() {
  document.getElementById('btn-tema')?.addEventListener('click', () => {
    const escuro = document.documentElement.getAttribute('data-bs-theme') !== 'dark';
    document.documentElement.setAttribute('data-bs-theme', escuro ? 'dark' : 'light');
    try { localStorage.setItem('bingo-theme', escuro ? 'dark' : 'light'); } catch { /* ok */ }
  });
}

// ------------------------------------------------------------------ "ocupado": esmaece e trava o app até terminar
/*
 * Sincronização, geração de PDFs e atualização usam a mesma tela: o app fica esmaecido e sem clique,
 * com um cartão por cima mostrando a etapa e a barra de progresso. Nada fica pela metade.
 */
const ocupado = {
  abrir(titulo: string, texto = '') {
    let el = document.getElementById('ocupado');
    if (!el) {
      el = document.createElement('div');
      el.id = 'ocupado';
      el.setAttribute('role', 'alertdialog');
      el.setAttribute('aria-modal', 'true');
      el.setAttribute('aria-live', 'polite');
      el.innerHTML = `<div class="ocupado-cartao">
        <div class="d-flex align-items-center gap-2 mb-2"><span class="spinner-border spinner-border-sm text-primary" aria-hidden="true"></span><strong id="ocupado-titulo"></strong></div>
        <div class="progress" role="progressbar" aria-label="Progresso"><div class="progress-bar progress-bar-striped progress-bar-animated" id="ocupado-barra" style="width:0%"></div></div>
        <div class="small text-secondary mt-2" id="ocupado-texto"></div></div>`;
      document.body.appendChild(el);
    }
    document.body.classList.add('esta-ocupado');
    el.querySelector('#ocupado-titulo')!.textContent = titulo;
    this.progresso(null, texto);
  },
  /** pct: 0–100, ou null = sem número (barra correndo). */
  progresso(pct: number | null, texto?: string) {
    const barra = document.getElementById('ocupado-barra') as HTMLElement | null;
    if (barra) barra.style.width = pct === null ? '100%' : `${Math.max(3, Math.min(100, pct))}%`;
    barra?.classList.toggle('opacity-50', pct === null);
    if (texto !== undefined) { const t = document.getElementById('ocupado-texto'); if (t) t.textContent = texto; }
  },
  fechar() {
    document.getElementById('ocupado')?.remove();
    document.body.classList.remove('esta-ocupado');
  },
};
const pct = (feitas: number, total: number) => (total ? Math.round((feitas / total) * 100) : 100);
const n = (v: number) => Number(v).toLocaleString('pt-BR');

// ------------------------------------------------------------------ início: login ou demonstração

async function iniciar(aviso?: string): Promise<void> {
  situacao = await invoke('situacao');
  if (situacao.logado && !aviso) return telaFestas();
  const motivo = aviso ?? (situacao.motivo?.codigo && situacao.motivo.codigo !== 'sem_login' ? situacao.motivo.message : '');

  app.innerHTML = `${topo()}
  <main class="container py-5" style="max-width:980px">
    ${motivo ? `<div class="alert alert-warning"><i class="fas fa-triangle-exclamation me-1"></i>${esc(motivo)}</div>` : ''}
    <div class="row g-4">
      <div class="col-md-6"><div class="card h-100"><div class="card-body p-4">
        <h1 class="h4">Entrar com a sua conta</h1>
        <p class="text-secondary small">Use o mesmo e-mail e senha do site. Este computador fica conectado à conta (até ${2} computadores).</p>
        <form id="form-login" novalidate>
          <div class="mb-3"><label class="form-label" for="email">E-mail</label>
            <input class="form-control form-control-lg" id="email" type="email" autocomplete="email" required></div>
          <div class="mb-3"><label class="form-label" for="senha">Senha</label>
            <div class="input-group"><input class="form-control form-control-lg" id="senha" type="password" autocomplete="current-password" required>
            <button type="button" class="btn btn-outline-secondary" id="olho" aria-label="Mostrar senha" aria-pressed="false"><i class="fas fa-eye"></i></button></div></div>
          <div class="mb-3 d-none" id="campo-codigo"><label class="form-label" for="codigo">Código do app autenticador</label>
            <input class="form-control form-control-lg text-center" id="codigo" inputmode="numeric" autocomplete="one-time-code" maxlength="11"></div>
          <button class="btn btn-primary btn-lg w-100" id="btn-entrar">Entrar</button>
        </form>
        <p class="small text-secondary mt-3 mb-0">Não tem conta? <a href="#" id="link-site">Veja os planos e fale com a gente</a>.</p>
      </div></div></div>
      <div class="col-md-6"><div class="card h-100"><div class="card-body p-4 d-flex flex-column">
        <h2 class="h4">Experimentar sem conta</h2>
        <p class="text-secondary">Demonstração gratuita, sem cadastro e sem internet: a festa de exemplo do site com <strong>20 cartelas e 6 prêmios</strong>, mais 2 rodadas extras (folha inteira e meia folha). As cartelas são as mesmas em qualquer computador: imprima os PDFs e teste com a família.</p>
        <ul class="small text-secondary"><li>Sorteio manual ou automático com suspense</li><li>Conferência de cartela e confirmação do vencedor</li><li>Modo telão (tela cheia)</li><li>PDFs prontos para imprimir, no modelo com cabeçalho, Pix, sua logo e patrocinadores</li></ul>
        <button class="btn btn-accent btn-lg mt-auto" id="btn-demo"><i class="fas fa-play me-1"></i> Abrir demonstração</button>
      </div></div></div>
    </div>
    <p class="text-center small text-secondary mt-4">Servidor: ${esc(situacao.servidor)}</p>
  </main>`;
  ligarTema();

  const olho = document.getElementById('olho')!;
  olho.addEventListener('click', () => {
    const s = document.getElementById('senha') as HTMLInputElement;
    const mostrar = s.type === 'password';
    s.type = mostrar ? 'text' : 'password';
    olho.setAttribute('aria-pressed', String(mostrar));
    olho.innerHTML = `<i class="fas fa-eye${mostrar ? '-slash' : ''}"></i>`;
  });
  document.getElementById('link-site')!.addEventListener('click', (e) => {
    e.preventDefault();
    openUrl(String(situacao.servidor).replace(/\/api\/desktop\/v1$/, '/#planos'));
  });
  document.getElementById('btn-demo')!.addEventListener('click', async () => {
    await chamar('iniciar_demo', { recomecar: false });
    abrirFesta(0);
  });

  document.getElementById('form-login')!.addEventListener('submit', async (e) => {
    e.preventDefault();
    const btn = document.getElementById('btn-entrar') as HTMLButtonElement;
    const codigo = (document.getElementById('codigo') as HTMLInputElement).value.trim();
    btn.disabled = true; btn.textContent = 'Entrando…';
    try {
      await invoke('login', {
        email: (document.getElementById('email') as HTMLInputElement).value.trim(),
        senha: (document.getElementById('senha') as HTMLInputElement).value,
        codigo: codigo || null,
      });
      sincronizouAoAbrir = false; // logou: baixa tudo de novo (festas, cartelas, imagens e patrocinadores)
      telaFestas();
    } catch (err) {
      const e2 = lerErro(err);
      if (e2.codigo === 'precisa_2fa') {
        document.getElementById('campo-codigo')!.classList.remove('d-none');
        (document.getElementById('codigo') as HTMLInputElement).focus();
        toast(e2.message, 'info');
      } else if (e2.codigo === 'limite_dispositivos') {
        const lista = (e2.dispositivos ?? []).map((d: Json) => `<li>${esc(d.nome)} (${esc(d.plataforma)}) — último uso ${fmtData(d.ultimo_contato_em)}</li>`).join('');
        Swal.fire({ icon: 'info', title: 'Limite de computadores', html: `${esc(e2.message)}<ul class="text-start mt-2">${lista}</ul>` });
      } else {
        toast(e2.message, 'error');
      }
    } finally { btn.disabled = false; btn.textContent = 'Entrar'; }
  });
}

// ------------------------------------------------------------------ festas da licença

async function telaFestas(): Promise<void> {
  situacao = await invoke('situacao');
  if (!situacao.logado) return iniciar();
  // renova a licença quando há internet (sem internet, segue com a salva até "valida_ate");
  // na 1ª vez desde que o app abriu (ou logo depois do login), sincroniza todas as festas
  desenhar();
  invoke('renovar').then(async () => {
    situacao = await invoke('situacao');
    desenhar();
    if (!sincronizouAoAbrir) {
      sincronizouAoAbrir = true;
      await sincronizarTudo();
      situacao = await invoke('situacao');
      desenhar();
      verificarAtualizacao();
    }
  }).catch((e) => {
    desenhar(true);
    if (lerErro(e).codigo === 'atualizar_app') verificarAtualizacao(true);
  });

  function desenhar(offline = false) {
    const lic = situacao.licenca;
    const festas = (lic.festas ?? []) as Json[];
    app.innerHTML = `${topo(`
        ${offline ? '<span class="offline-pill small"><i class="fas fa-wifi me-1"></i>offline</span>' : ''}
        <span class="small text-secondary">${esc(lic.conta.nome)}</span>
        <button class="btn btn-sm btn-outline-danger" id="btn-sair"><i class="fas fa-right-from-bracket me-1"></i>Sair</button>`)}
    <main class="container py-4" style="max-width:980px">
      <div class="d-flex flex-wrap justify-content-between align-items-end gap-2 mb-3">
        <div><h1 class="h3 mb-0">Suas festas</h1>
          <div class="status-linha text-secondary">Licença válida neste computador até <strong>${fmtData(lic.valida_ate)}</strong> sem internet.
          ${situacao.pendentes ? ` · <span class="text-warning">${situacao.pendentes} ação(ões) do sorteio aguardando envio</span>` : ''}</div></div>
      </div>
      <div class="d-grid gap-3">
        ${festas.length ? festas.map((f) => `
          <div class="festa-item d-flex flex-wrap justify-content-between align-items-center gap-2">
            <div><div class="fw-semibold fs-5">${esc(f.nome)}</div>
              <div class="small text-secondary">${f.data ? new Date(f.data + 'T12:00').toLocaleDateString('pt-BR') : ''} · plano ${esc(f.plano.nome)} · ${Number(f.total_cartelas).toLocaleString('pt-BR')} cartelas
              ${f.pode_sortear ? '' : ' · <span class="text-danger">fora do plano: sorteio bloqueado</span>'}</div></div>
            <button class="btn btn-primary btn-abrir" data-id="${f.id}"><i class="fas fa-play me-1"></i>Abrir</button>
          </div>`).join('') : '<div class="alert alert-secondary">Nenhuma festa em uso nesta conta. Crie uma no site.</div>'}
        <div class="festa-item d-flex justify-content-between align-items-center gap-2">
          <div><div class="fw-semibold">Demonstração</div><div class="small text-secondary">20 cartelas + 2 rodadas extras de teste, iguais em qualquer computador</div></div>
          <button class="btn btn-outline-primary" id="btn-demo2">Abrir</button>
        </div>
      </div>
    </main>`;
    ligarTema();
    document.getElementById('btn-sair')!.addEventListener('click', async () => {
      const r = await Swal.fire({ icon: 'warning', title: 'Sair da conta neste computador?', text: 'Os dados sincronizados são apagados deste computador. Os PDFs salvos continuam onde estão.' + (situacao.pendentes ? ' Atenção: há ações do sorteio ainda não enviadas.' : ''), showCancelButton: true, confirmButtonText: 'Sair', cancelButtonText: 'Cancelar' });
      if (r.isConfirmed) { await invoke('sair'); iniciar(); }
    });
    document.getElementById('btn-demo2')!.addEventListener('click', async () => { await chamar('iniciar_demo', { recomecar: false }); abrirFesta(0); });
    document.querySelectorAll<HTMLButtonElement>('.btn-abrir').forEach((b) => b.addEventListener('click', () => abrirFesta(Number(b.dataset.id), true)));
  }
}

let sincronizouAoAbrir = false;

/** Texto da etapa da sincronização (evento "progresso" do app). */
function textoEtapa(p: Json): [number | null, string] {
  if (p.etapa === 'enviando') return [null, p.total ? `Enviando ${n(p.total)} ação(ões) do sorteio feitas sem internet…` : 'Conferindo o sorteio com o servidor…'];
  if (p.etapa === 'imagens') return [pct(p.feitas, p.total), `Baixando imagens e patrocinadores: ${n(p.feitas)} de ${n(p.total)}`];
  return [pct(p.feitas, p.total), `Baixando cartelas: ${n(p.feitas)} de ${n(p.total)} (${pct(p.feitas, p.total)}%)`];
}

/**
 * Sincroniza uma festa com a tela "ocupado". Envia o que foi feito sem internet, baixa festa, cartelas,
 * imagens e patrocinadores. titulo/prefixo: usados quando várias festas são sincronizadas em sequência.
 */
async function sincronizarFesta(id: number, silencioso = false, titulo = 'Sincronizando com o servidor…', prefixo = ''): Promise<boolean> {
  ocupado.abrir(titulo, prefixo + 'Conectando…');
  const desligar = await listen<Json>('progresso', (ev) => {
    const [p, texto] = textoEtapa(ev.payload);
    ocupado.progresso(p, prefixo + texto);
  });
  try {
    const r: Json = await chamar('sincronizar', { festaId: id });
    if (r.recusado) Swal.fire({ icon: 'warning', title: 'O servidor recusou uma ação do sorteio', text: `${r.recusado} O sorteio foi atualizado com o que está registrado na nuvem.` });
    else if (!silencioso) toast(r.enviados ? `Sincronizado: ${r.enviados} ação(ões) enviada(s).` : 'Sincronizado.', 'success');
    return true;
  } catch (e) {
    const err = e as Json;
    if (err?.codigo === 'sem_internet') { if (!silencioso) toast(err.message, 'info'); return false; }
    if (err?.codigo === 'atualizar_app') { ocupado.fechar(); verificarAtualizacao(true); return false; }
    ocupado.fechar();
    mostrarErro(err); return false;
  } finally { desligar(); ocupado.fechar(); }
}

/** Ao abrir o app (e logo depois do login): sincroniza todas as festas da licença, uma por vez. */
async function sincronizarTudo(): Promise<void> {
  const festas = ((situacao?.licenca?.festas ?? []) as Json[]);
  for (const [i, f] of festas.entries()) {
    const prefixo = festas.length > 1 ? `${f.nome} (${i + 1} de ${festas.length}) · ` : `${f.nome} · `;
    const ok = await sincronizarFesta(f.id, true, 'Sincronizando suas festas…', prefixo);
    if (!ok) break; // sem internet: segue com o que já está no computador
  }
}

async function abrirFesta(id: number, baixar = false) {
  festaId = id;
  if (baixar) {
    const ok = await sincronizarFesta(id, true);
    if (!ok) {
      try { await invoke('estado_festa', { festaId: id }); } catch { return toast('Sem internet e esta festa ainda não foi baixada neste computador.', 'error'); }
      toast('Sem internet: usando os dados já baixados.', 'info');
    }
  }
  try { estado = await chamar('estado_festa', { festaId: id }); } catch (e) { return mostrarErro(e); }
  rodadaId = estado.rodadas?.[0]?.id ?? null;
  telaSorteio();
  avisarArquivosSumidos();
  sugerirTelao();
}

/** Sugere o telão em tela cheia ao abrir a festa (dá para não mostrar de novo neste computador). */
function sugerirTelao() {
  try { if (localStorage.getItem('bingo-sugerir-telao') === 'off') return; } catch { /* ok */ }
  Swal.fire({
    icon: 'info',
    title: 'Vai projetar o sorteio?',
    html: 'Para a melhor experiência do público, use o <strong>telão em tela cheia</strong>.<br>Para voltar, aperte Esc.',
    input: 'checkbox',
    inputPlaceholder: 'Não mostrar de novo neste computador',
    showCancelButton: true,
    confirmButtonText: '<i class="fas fa-expand me-1"></i> Abrir em tela cheia',
    cancelButtonText: 'Agora não',
    reverseButtons: true,
    willClose: () => {
      if ((Swal.getInput() as HTMLInputElement | null)?.checked) { try { localStorage.setItem('bingo-sugerir-telao', 'off'); } catch { /* ok */ } }
    },
  }).then((r: { isConfirmed: boolean }) => { if (r.isConfirmed) entrarTelao(); });
}

/** Ao abrir a festa: confere se os PDFs gerados neste computador continuam no lugar e iguais. */
async function avisarArquivosSumidos() {
  try {
    const lista: Json[] = await chamar('listar_pdfs', { festaId });
    const problemas = lista.filter((p) => p.situacao !== 'ok');
    if (!problemas.length) return;
    const r = await Swal.fire({
      icon: 'warning', title: `${problemas.length} arquivo(s) de cartelas não encontrado(s) ou alterado(s)`,
      html: `<ul class="text-start small">${problemas.slice(0, 6).map((p) => `<li><strong>${esc(p.nome)}</strong> — ${p.situacao === 'sumiu' ? 'não está mais em' : 'foi alterado em'} <code>${esc(p.caminho)}</code></li>`).join('')}</ul>
             Você pode gerar de novo: as cartelas saem iguais às originais.`,
      showCancelButton: true, confirmButtonText: 'Ver os PDFs', cancelButtonText: 'Depois',
    });
    if (r.isConfirmed) telaPdfs();
  } catch { /* sem PDFs */ }
}

// ------------------------------------------------------------------ cartelas e PDFs (só no desktop)

const kb = (b: number) => (b > 1048576 ? `${(b / 1048576).toLocaleString('pt-BR', { maximumFractionDigits: 1 })} MB` : `${Math.round(b / 1024)} KB`);

async function telaPdfs() {
  const demo = festaId === 0;
  estado = await chamar('estado_festa', { festaId });
  const rodadas = estado.rodadas as Json[];
  app.innerHTML = `${topo(`<button class="btn btn-sm btn-outline-secondary" id="btn-voltar-sorteio"><i class="fas fa-arrow-left me-1"></i>Sorteio</button>`)}
  <main class="container py-4" style="max-width:1100px">
    <h1 class="h3 mb-1">${esc(estado.festa.nome)} — cartelas e PDFs</h1>
    ${demo ? `<p class="text-secondary">Na demonstração, as cartelas saem no <strong>modelo de exemplo do site</strong>: cabeçalho, Pix, "SUA LOGO" e patrocinadores de mentira. Com a sua conta, entram o cabeçalho, a logo, o Pix e os patrocinadores da sua festa, montados no site.</p>` : ''}
    <p class="text-secondary">Os PDFs são gerados <strong>neste computador</strong>, com o layout montado no site, e salvos na pasta que você escolher (sugestão: <em>Documentos › Bingo Digital › ${esc(estado.festa.nome)} › PDFs</em>, junto das imagens e dos patrocinadores da festa). O servidor guarda só o registro de cada arquivo. Se um arquivo sumir, gere de novo: as cartelas saem iguais.</p>
    <div class="row g-3">
      ${demo ? '' : `<div class="col-lg-5"><div class="card h-100"><div class="card-body">
        <h2 class="h6 fw-semibold"><i class="fas fa-wand-magic-sparkles me-1"></i>Gerar mais cartelas</h2>
        <p class="small text-secondary">${Number(estado.total_local).toLocaleString('pt-BR')} cartelas na festa · plano ${esc(estado.festa.plano?.nome ?? '')} (até ${Number(estado.festa.plano?.limite_cartelas ?? 0).toLocaleString('pt-BR')}). Os números são sorteados no servidor.</p>
        <div class="input-group"><input type="number" min="1" max="20000" class="form-control" id="qtd" placeholder="Quantidade"><button class="btn btn-primary" id="btn-gerar-cartelas">Gerar</button></div>
      </div></div></div>`}
      <div class="${demo ? 'col-12' : 'col-lg-7'}"><div class="card h-100"><div class="card-body">
        <h2 class="h6 fw-semibold"><i class="fas fa-file-pdf me-1"></i>Gerar PDFs</h2>
        ${demo ? `<div class="d-flex flex-wrap align-items-center gap-3 mb-3 pb-3 border-bottom">
          <ul class="small text-secondary mb-0 flex-grow-1">
            <li><strong>Principal</strong>: 20 cartelas em folha inteira, uma grade para cada um dos 6 prêmios</li>
            <li><strong>Extra 1</strong>: 4 cartelas em folha inteira, com "EXTRA 1" na lateral</li>
            <li><strong>Extra 2</strong>: 4 cartelas em meia folha (2 folhas, com linha de corte)</li>
          </ul>
          <button class="btn btn-accent" id="btn-gerar-demo"><i class="fas fa-file-pdf me-1"></i>Gerar os 3 PDFs de exemplo</button>
        </div>
        <p class="small text-secondary mb-2">Ou gere uma rodada só, escolhendo o formato:</p>` : ''}
        <div class="d-flex flex-wrap gap-2">
          <select class="form-select" id="pdf-rodada" style="max-width:280px">${rodadas.map((r) => `<option value="${r.id}">${esc(r.nome)} (${r.tipo === 'principal' ? 'todas' : r.cartela_inicio + '–' + r.cartela_fim}${r.embaralhar && r.tipo !== 'principal' ? ', embaralhada' : ''})</option>`).join('')}</select>
          <select class="form-select" id="pdf-formato" style="max-width:260px" aria-label="Formato das cartelas da rodada extra">
            <option value="folha">Folha inteira (1 por folha)</option>
            <option value="meia_folha">Meia folha (2 por folha, com linha de corte)</option>
          </select>
          <select class="form-select" id="pdf-qtd" style="max-width:220px"><option value="500">500 por arquivo</option><option value="1000">1000 por arquivo</option><option value="250">250 por arquivo</option></select>
          <button class="btn btn-accent" id="btn-gerar-pdfs"><i class="fas fa-file-pdf me-1"></i>Gerar PDFs</button>
        </div>
      </div></div></div>
    </div>
    <div class="card mt-3"><div class="card-body">
      <h2 class="h6 fw-semibold mb-3"><i class="fas fa-list me-1"></i>PDFs gerados neste computador</h2>
      <div id="lista-pdfs" class="table-responsive"></div>
    </div></div>
  </main>`;
  ligarTema();
  document.getElementById('btn-voltar-sorteio')!.addEventListener('click', () => { telaSorteio(); });

  document.getElementById('btn-gerar-cartelas')?.addEventListener('click', async () => {
    const qtd = Number((document.getElementById('qtd') as HTMLInputElement).value);
    if (!qtd) return toast('Informe a quantidade.');
    try {
      const r: Json = await chamar('gerar_cartelas', { festaId, quantidade: qtd });
      toast(r.message, 'success');
      if (await sincronizarFesta(festaId!, true)) telaPdfs();
    } catch (e) { mostrarErro(e); }
  });

  // formato: a Principal é sempre folha inteira; na extra, escolhe aqui (começa no padrão da rodada)
  const selRodada = document.getElementById('pdf-rodada') as HTMLSelectElement;
  const selFormato = document.getElementById('pdf-formato') as HTMLSelectElement;
  const aplicarFormato = () => {
    const r = rodadas.find((x) => x.id === Number(selRodada.value));
    const extra = r && r.tipo !== 'principal';
    selFormato.hidden = !extra;
    if (extra) selFormato.value = r.formato === 'meia_folha' ? 'meia_folha' : 'folha';
  };
  selRodada.addEventListener('change', aplicarFormato);
  aplicarFormato();

  /** Gera os PDFs de uma ou mais rodadas (cada uma no seu formato) na pasta escolhida. */
  const gerar = async (lista: { rodadaId: number; formato: string | null }[]) => {
    // sugestão: Documentos/Bingo Digital/<festa>/PDFs (o cliente pode escolher outra)
    let sugestao: string | undefined;
    try { sugestao = (await chamar<string | null>('pasta_festa', { festaId, sub: 'PDFs' })) ?? undefined; } catch { /* sem pasta Documentos */ }
    const pasta = await escolherPasta({ directory: true, title: 'Onde salvar os PDFs das cartelas?', defaultPath: sugestao });
    if (!pasta) return;
    ocupado.abrir('Gerando os PDFs das cartelas…', 'Preparando…');
    const desligar = await listen<Json>('progresso_pdf', (ev) => {
      const p = ev.payload;
      if (p.etapa === 'imagens') return ocupado.progresso(null, 'Conferindo as imagens da festa (cabeçalho, logo, patrocinadores)…');
      ocupado.progresso(pct(p.feitos, p.total), `Arquivo ${p.feitos} de ${p.total}: ${p.arquivo}`);
    });
    try {
      const caminhos: string[] = [];
      for (const item of lista) {
        const r: Json = await chamar('gerar_pdfs', {
          festaId, rodadaId: item.rodadaId, porArquivo: Number((document.getElementById('pdf-qtd') as HTMLSelectElement).value),
          pasta: String(pasta), formato: item.formato,
        });
        caminhos.push(...(r.caminhos ?? []));
      }
      ocupado.fechar();
      const abrir = await Swal.fire({ icon: 'success', title: `${caminhos.length} arquivo(s) de cartelas salvo(s)`, text: String(pasta),
        showCancelButton: true, confirmButtonText: '<i class="fas fa-folder-open me-1"></i> Abrir a pasta', cancelButtonText: 'Fechar' });
      if (abrir.isConfirmed && caminhos[0]) revealItemInDir(caminhos[0]).catch(() => {});
      desenharListaPdfs();
    } catch (e) { ocupado.fechar(); mostrarErro(e); } finally { desligar(); ocupado.fechar(); }
  };
  document.getElementById('btn-gerar-pdfs')!.addEventListener('click', () => gerar([
    { rodadaId: Number(selRodada.value), formato: selFormato.hidden ? null : selFormato.value },
  ]));
  // demonstração: os 3 modelos de uma vez, cada rodada no seu formato padrão
  document.getElementById('btn-gerar-demo')?.addEventListener('click', () => gerar(rodadas.map((r) => ({ rodadaId: Number(r.id), formato: null }))));
  desenharListaPdfs();
}

async function desenharListaPdfs() {
  const lista: Json[] = await chamar('listar_pdfs', { festaId });
  const nomeRodada = (id: number) => estado.rodadas.find((r: Json) => r.id === id)?.nome ?? '—';
  const selo = (s: string) => s === 'ok' ? '<span class="badge text-bg-success">no lugar</span>'
    : s === 'sumiu' ? '<span class="badge text-bg-danger">não encontrado</span>' : '<span class="badge text-bg-warning">alterado</span>';
  document.getElementById('lista-pdfs')!.innerHTML = lista.length ? `<table class="table table-sm align-middle mb-0">
    <thead><tr><th>Arquivo</th><th>Rodada</th><th>Cartelas</th><th>Páginas</th><th>Tamanho</th><th>Situação</th><th class="text-end">Ações</th></tr></thead><tbody>
    ${lista.slice().reverse().map((p) => `<tr>
      <td class="small"><div class="fw-semibold">${esc(p.nome)}</div><div class="text-secondary text-break">${esc(p.caminho)}</div>${p.detalhe ? `<div class="text-secondary"><i class="fas fa-scissors me-1"></i>${esc(p.detalhe)}</div>` : ''}</td>
      <td>${esc(nomeRodada(p.rodada_id))}</td>
      <td class="text-nowrap">${String(p.primeira).padStart(4, '0')}–${String(p.ultima).padStart(4, '0')}</td>
      <td>${p.paginas}</td><td class="text-nowrap">${kb(p.tamanho)}</td>
      <td>${selo(p.situacao)} ${p.na_nuvem ? '<i class="fas fa-cloud text-secondary" title="Registrado na nuvem"></i>' : ''}</td>
      <td class="text-end text-nowrap">
        ${p.situacao === 'ok' ? `<button class="btn btn-sm btn-outline-primary" data-mostrar="${esc(p.caminho)}" title="Mostrar na pasta"><i class="fas fa-folder-open"></i></button>` : `<button class="btn btn-sm btn-warning" data-regerar="${p.id}"><i class="fas fa-rotate me-1"></i>Gerar de novo</button>`}
        <button class="btn btn-sm btn-outline-danger" data-esquecer="${p.id}" title="Remover da lista"><i class="fas fa-xmark"></i></button>
      </td></tr>`).join('')}</tbody></table>`
    : '<span class="text-secondary">Nenhum PDF gerado neste computador ainda.</span>';

  document.querySelectorAll<HTMLButtonElement>('[data-mostrar]').forEach((b) => b.addEventListener('click', () => revealItemInDir(b.dataset.mostrar!).catch(() => toast('Não foi possível abrir a pasta.', 'error'))));
  document.querySelectorAll<HTMLButtonElement>('[data-regerar]').forEach((b) => b.addEventListener('click', async () => {
    try {
      const r: Json = await chamar('regerar_pdf', { id: Number(b.dataset.regerar) });
      toast(r.igual_ao_original ? 'Arquivo gerado de novo, idêntico ao original.' : 'Arquivo gerado de novo.', 'success');
      desenharListaPdfs();
    } catch (e) { mostrarErro(e); }
  }));
  document.querySelectorAll<HTMLButtonElement>('[data-esquecer]').forEach((b) => b.addEventListener('click', async () => {
    const r = await Swal.fire({ icon: 'warning', title: 'Remover da lista?', text: 'O registro sai deste computador e da nuvem. O arquivo em si não é apagado.', showCancelButton: true, confirmButtonText: 'Remover', cancelButtonText: 'Cancelar' });
    if (!r.isConfirmed) return;
    try { await chamar('esquecer_pdf', { id: Number(b.dataset.esquecer) }); desenharListaPdfs(); } catch (e) { mostrarErro(e); }
  }));
}

// ------------------------------------------------------------------ sorteio

const rodadaAtual = () => estado.rodadas.find((r: Json) => r.id === rodadaId);
/** Prêmio em jogo: o escolhido pelo operador (qualquer ordem); se só sobrou um aberto, é ele. */
/** Prêmio no telão: só informativo, opcional (quem conduz o bingo decide prêmios e ganhadores). */
const premioExibido = () => (rodadaAtual()?.premios ?? []).find((p: Json) => p.id === rodadaAtual()?.premio_atual_id) ?? null;
/** O quadro é da rodada: pedras na ordem em que saíram. */
const pedrasOrdem = (): Json[] => [...(rodadaAtual()?.pedras ?? [])].sort((a, b) => a.ordem - b.ordem);
const rotulo = (p: Json) => (/^\d/.test(p.titulo) ? p.titulo : `${p.ordem}º ${p.titulo}`);

async function recarregar() {
  estado = await chamar('estado_festa', { festaId });
  desenharSorteio();
}

function telaSorteio() {
  const demo = festaId === 0;
  app.innerHTML = `${topo(`
      <span class="small text-secondary" id="pendentes"></span>
      ${demo ? '<button class="btn btn-sm btn-outline-secondary" id="btn-novas"><i class="fas fa-rotate-left me-1"></i>Recomeçar sorteio</button>' : '<button class="btn btn-sm btn-outline-primary" id="btn-sync"><i class="fas fa-rotate me-1"></i>Sincronizar</button>'}
      <button class="btn btn-sm btn-outline-primary" id="btn-pdfs"><i class="fas fa-file-pdf me-1"></i>Cartelas e PDFs</button>
      ${demo ? '' : '<button class="btn btn-sm btn-outline-primary" id="btn-registro"><i class="fas fa-list-check me-1"></i>Registro</button>'}
      <button class="btn btn-sm btn-primary" id="btn-telao"><i class="fas fa-expand me-1"></i>Telão</button>
      <button class="btn btn-sm btn-outline-secondary" id="btn-voltar"><i class="fas fa-arrow-left me-1"></i>Festas</button>`)}
  <div class="container-fluid py-3">
    <div class="d-flex flex-wrap align-items-center gap-2 mb-2">
      <h1 class="h4 mb-0 me-2">${esc(estado.festa.nome)}</h1>
      <div class="nav nav-pills gap-1 so-operador" id="rodadas"></div>
    </div>
    <div class="card mb-3" id="card-premios"><div class="card-body py-2">
      <div class="d-flex flex-wrap align-items-center gap-2" id="premios"></div>
    </div></div>
    <div class="row g-3">
      <div class="col-lg-8">
        <div class="card"><div class="card-body"><div class="draw-board" id="tabuleiro"></div>
          <div class="small text-secondary mt-2 so-operador"><i class="fas fa-circle-info me-1"></i>Clique numa pedra para marcar; clique de novo para desmarcar um engano. Entre um prêmio e outro, limpe o quadro.</div></div></div>
        <div class="so-telao mt-3" id="painel-divulgacao">${divulgacao()}</div>
      </div>
      <div class="col-lg-4">
        <div class="card h-100"><div class="card-body text-center d-flex flex-column">
          <span class="page-eyebrow mb-2">Última pedra</span>
          <div class="bola-area mb-3"><div class="bingo-ball bingo-ball--giant mx-auto" id="bola"></div></div>
          <div class="row g-2 mb-2"><div class="col-6"><div class="draw-stat"><div class="ds-value" id="st-saiu">0</div><div class="ds-label">Sorteadas</div></div></div>
            <div class="col-6"><div class="draw-stat is-remaining"><div class="ds-value" id="st-resta">75</div><div class="ds-label">Restam</div></div></div></div>
          <div class="so-telao"><form class="conferir-telao" id="form-conferir-telao" autocomplete="off">
            <input type="text" class="form-control" id="cod-telao" placeholder="Conferir: número ou código" aria-label="Número ou código da cartela">
            <button type="submit" class="btn btn-primary"><i class="fas fa-magnifying-glass me-1"></i>Conferir</button>
          </form></div>
          <div class="so-telao"><div id="painel-top5" class="top5-telao">
            <div class="top5-compacto-titulo mb-2"><i class="fas fa-fire text-danger me-1"></i>Quase batendo</div>
            <div class="top5-list" id="top5"></div>
          </div></div>
          <div class="d-grid gap-2 mt-auto so-operador">
            ${estado.festa.sorteio_automatico ? '<button class="btn btn-accent btn-lg" id="btn-sortear"><i class="fas fa-shuffle me-1"></i>Sortear pedra <small>(espaço)</small></button>' : ''}
            <button class="btn btn-outline-primary" id="btn-remover"><i class="fas fa-rotate-left me-1"></i>Remover última</button>
            <button class="btn btn-outline-danger" id="btn-limpar"><i class="fas fa-broom me-1"></i>Limpar quadro (próximo prêmio)</button>
          </div>
          <div class="text-start border-top pt-3 mt-3 so-operador" id="opcoes-exibicao">
            ${temTop5Patrocinadores() ? opcaoSwitch('top5', 'fa-fire', 'Top 5 (quase batendo) no telão') : ''}
            ${opcaoSwitch('narrar', 'fa-volume-high', 'Narrar pedra (voz)')}
            ${temTop5Patrocinadores() ? opcaoSwitch('patrocinadores', 'fa-handshake', 'Patrocinadores no telão') : '<p class="small text-secondary mb-0 mt-1"><i class="fas fa-lock me-1"></i>Top 5 e patrocinadores no telão: a partir do plano Comunidade.</p>'}
          </div>
        </div></div>
      </div>
      <div class="col-12" id="painel-ordem"><div class="card"><div class="card-body">
        <h6 class="fw-semibold mb-3"><i class="fas fa-list-ol me-1"></i>Ordem do sorteio</h6>
        <div class="order-list" id="ordem"></div>
      </div></div></div>
      <div class="col-12" id="painel-patrocinadores" hidden><div class="card"><div class="card-body py-3">
        <div class="small text-secondary text-center text-uppercase fw-semibold mb-2" style="letter-spacing:.08em;">Patrocinadores</div>
        <div class="sponsor-marquee"><div class="sponsor-track" id="patrocinio-trilho"></div></div>
      </div></div></div>
      <div class="col-12 so-operador"><div class="card"><div class="card-body">
        <label class="form-label fw-semibold" for="cod"><i class="fas fa-magnifying-glass me-1"></i>Conferir cartela</label>
        <div class="input-group input-group-lg" style="max-width:460px"><input class="form-control" id="cod" autocomplete="off" placeholder="Número (0042) ou código (7K3QX)"><button class="btn btn-primary" id="btn-conferir">Conferir</button></div>
      </div></div></div>
    </div>
  </div>
  <button class="btn btn-dark position-fixed bottom-0 end-0 m-3 d-none" id="btn-sair-telao"><i class="fas fa-compress me-1"></i>Sair do telão</button>`;
  ligarTema();

  const linhas = LETRAS.map((L, i) => `<div class="draw-row row-${L.toLowerCase()}"><div class="draw-row-label">${L}</div>${Array.from({ length: 15 }, (_, k) => i * 15 + k + 1)
    .map((n) => `<button class="draw-cell" data-n="${n}" aria-label="${L} ${n}">${String(n).padStart(2, '0')}</button>`).join('')}</div>`).join('');
  document.getElementById('tabuleiro')!.innerHTML = linhas;

  document.getElementById('btn-voltar')!.addEventListener('click', () => { sairTelao(); demo ? iniciarOuFestas() : telaFestas(); });
  document.getElementById('btn-sync')?.addEventListener('click', async () => { if (await sincronizarFesta(festaId!)) recarregar(); });
  document.getElementById('btn-novas')?.addEventListener('click', async () => {
    const r = await Swal.fire({ icon: 'question', title: 'Recomeçar o sorteio?', text: 'As pedras e os vencedores da demonstração são apagados. As cartelas continuam as mesmas.', showCancelButton: true, confirmButtonText: 'Recomeçar', cancelButtonText: 'Cancelar' });
    if (!r.isConfirmed) return;
    await chamar('iniciar_demo', { recomecar: true });
    recarregar();
    toast('Sorteio da demonstração recomeçado.', 'success');
  });
  document.getElementById('btn-telao')!.addEventListener('click', entrarTelao);
  document.getElementById('btn-pdfs')!.addEventListener('click', () => telaPdfs());
  document.getElementById('btn-registro')?.addEventListener('click', () => { sairTelao(); telaRegistro(); });
  ligarOpcoes();
  carregarPatrocinadores();
  document.getElementById('btn-sair-telao')!.addEventListener('click', sairTelao);
  document.getElementById('tabuleiro')!.addEventListener('click', clicarPedra);
  document.getElementById('btn-sortear')?.addEventListener('click', sortear);
  document.getElementById('btn-remover')!.addEventListener('click', () => { const u = pedrasOrdem().slice(-1)[0]; if (u) remover(u.numero, u.automatico); });
  document.getElementById('btn-limpar')!.addEventListener('click', limpar);
  document.getElementById('btn-conferir')!.addEventListener('click', () => conferir());
  document.getElementById('cod')!.addEventListener('keydown', (e) => { if ((e as KeyboardEvent).key === 'Enter') conferir(); });
  document.getElementById('form-conferir-telao')!.addEventListener('submit', (e) => { e.preventDefault(); conferir('cod-telao'); });
  document.onkeydown = (e) => {
    const campo = ['INPUT', 'TEXTAREA', 'BUTTON'].includes((document.activeElement as HTMLElement)?.tagName);
    if (e.code === 'Space' && !campo && !document.querySelector('.swal2-container') && estado.festa.sorteio_automatico) { e.preventDefault(); sortear(); }
    if (e.key === 'Escape' && document.body.classList.contains('telao') && !document.querySelector('.swal2-container')) sairTelao();
  };
  desenharSorteio();
}

// ------------------------------------------------------------------ divulgação do Bingo Digital (só no telão)

/** Endereço do site (o servidor da API sem o caminho da API). */
const enderecoSite = (): string => String(situacao?.servidor ?? '').replace(/\/api\/desktop\/v1\/?$/, '');

/** QR code em SVG gerado no próprio app (funciona offline). */
function qrSvg(texto: string): string {
  const qr = qrcode(0, 'M');
  qr.addData(texto);
  qr.make();
  return qr.createSvgTag({ cellSize: 4, margin: 0, scalable: true });
}

function divulgacao(): string {
  const site = enderecoSite();
  if (!site) return '';
  return `<div class="card divulgacao"><div class="card-body d-flex align-items-center gap-4">
    <div class="app-marca divulgacao-marca"><span class="bola">B</span>Bingo Digital</div>
    <div class="flex-grow-1">
      <div class="divulgacao-titulo">Faça o bingo da sua festa também!</div>
      <div class="divulgacao-texto">Cartelas, sorteio no telão e conferência na hora. Comece grátis em <strong>${esc(site.replace(/^https?:\/\//, ''))}</strong></div>
    </div>
    <div class="divulgacao-qr" aria-hidden="true">${qrSvg(site)}</div>
  </div></div>`;
}

// ------------------------------------------------------------------ opções de exibição (por computador)

const OPCOES_PADRAO: Record<string, boolean> = { top5: true, narrar: true, premio: true, patrocinadores: true };
/** Top 5 e patrocinadores no telão: só nos planos que têm (dado sincronizado da licença). */
function temTop5Patrocinadores(): boolean {
  return estado?.festa?.plano?.top5_patrocinadores !== false;
}
function opcao(nome: string): boolean {
  if ((nome === 'top5' || nome === 'patrocinadores') && !temTop5Patrocinadores()) return false;
  try { const v = localStorage.getItem('bingo-opcao-' + nome); return v === null ? OPCOES_PADRAO[nome] : v === '1'; } catch { return OPCOES_PADRAO[nome]; }
}
function opcaoSwitch(nome: string, icone: string, texto: string): string {
  return `<div class="form-check form-switch mb-1"><input class="form-check-input" type="checkbox" role="switch" id="op-${nome}" data-opcao="${nome}" ${opcao(nome) ? 'checked' : ''}>
    <label class="form-check-label small" for="op-${nome}"><i class="fas ${icone} me-1"></i>${texto}</label></div>`;
}
/** Carrossel dos patrocinadores (imagens baixadas na sincronização; funciona sem internet). */
let patrocinadoresFesta: Json[] = [];
async function carregarPatrocinadores() {
  try { patrocinadoresFesta = await chamar('patrocinadores', { festaId }); } catch { patrocinadoresFesta = []; }
  const trilho = document.getElementById('patrocinio-trilho');
  if (!trilho) return;
  const lista = patrocinadoresFesta;
  if (lista.length) {
    // repete até encher a faixa e duplica tudo: a 2ª metade continua a 1ª (rolagem sem emenda)
    const repeticoes = Math.max(1, Math.ceil(8 / lista.length));
    const uma = Array.from({ length: repeticoes }, () => lista).flat();
    trilho.innerHTML = [...uma, ...uma].map((p, i) => `<div class="sponsor-item" ${i >= lista.length ? 'aria-hidden="true"' : ''}>
      <img src="${p.imagem}" alt="${i >= lista.length ? '' : esc(p.nome ?? 'Patrocinador')}"></div>`).join('');
    (trilho.closest('.sponsor-marquee') as HTMLElement).style.setProperty('--duracao', `${Math.max(25, uma.length * 4)}s`);
  }
  aplicarOpcoes();
  agendarAjuste();
}

function aplicarOpcoes() {
  const pat = document.getElementById('painel-patrocinadores');
  if (pat) pat.hidden = !opcao('patrocinadores') || !patrocinadoresFesta.length;
  const top5 = document.getElementById('painel-top5');
  if (top5) top5.style.display = opcao('top5') ? '' : 'none';
}
function ligarOpcoes() {
  document.querySelectorAll<HTMLInputElement>('[data-opcao]').forEach((el) => el.addEventListener('change', () => {
    try { localStorage.setItem('bingo-opcao-' + el.dataset.opcao, el.checked ? '1' : '0'); } catch { /* ok */ }
    aplicarOpcoes();
    if (el.dataset.opcao === 'top5' && el.checked) carregarRanking();
  }));
  aplicarOpcoes();
}

// ------------------------------------------------------------------ registro do sorteio (administrativo)

const ESTILO_EVENTO: Record<string, [string, string]> = {
  pedra: ['fa-circle', 'text-secondary'], info: ['fa-tv', 'text-primary'], correcao: ['fa-triangle-exclamation', 'text-warning'],
  fim: ['fa-broom', 'text-success'],
};
const NOME_ACAO: Record<string, string> = {
  pedra: 'Pedra', remover: 'Pedra removida', limpar: 'Quadro limpo', escolher: 'Prêmio no telão',
};

async function telaRegistro() {
  app.innerHTML = `${topo(`<button class="btn btn-sm btn-outline-secondary" id="btn-voltar-sorteio"><i class="fas fa-arrow-left me-1"></i>Sorteio</button>`)}
  <main class="container py-4" style="max-width:1000px">
    <span class="page-eyebrow">Uso administrativo</span>
    <h1 class="h3 mb-1">Registro do sorteio — ${esc(estado.festa.nome)}</h1>
    <p class="text-secondary">Todas as pedras de cada rodada, na ordem, com as pedras removidas e os quadros limpos. Esta tela não aparece no telão.</p>
    <div id="registro"><div class="text-secondary"><span class="spinner-border spinner-border-sm me-2"></span>Carregando…</div></div>
  </main>`;
  ligarTema();
  document.getElementById('btn-voltar-sorteio')!.addEventListener('click', () => telaSorteio());

  let r: Json;
  try { r = await chamar('registro', { festaId }); } catch (e) { mostrarErro(e); return; }
  const hora = (iso?: string) => (iso ? new Date(iso).toLocaleString('pt-BR', { day: '2-digit', month: '2-digit', hour: '2-digit', minute: '2-digit', second: '2-digit' }) : '');
  const linha = (e: Json) => {
    const [icone, cor] = ESTILO_EVENTO[e.tipo] ?? ESTILO_EVENTO.info;
    return `<li class="d-flex gap-2 py-1 border-bottom ${e.tipo === 'pedra' ? 'text-secondary' : ''}">
      <span class="text-nowrap font-monospace">${hora(e.em)}</span><i class="fas ${icone} ${cor} mt-1" style="font-size:${e.tipo === 'pedra' ? '.5rem' : '.85rem'}"></i>
      <span><strong class="${e.tipo === 'pedra' ? 'fw-normal' : ''}">${esc(e.titulo)}</strong>${e.detalhe ? ' — ' + esc(e.detalhe) : ''}
      ${e.quem ? `<span class="text-secondary"> · ${esc(e.quem)}</span>` : ''}</span></li>`;
  };

  let html = '';
  const pend = (r.pendentes ?? []) as Json[];
  if (pend.length) {
    html += `<div class="card border-warning mb-3"><div class="card-body"><h2 class="h6 fw-semibold"><i class="fas fa-cloud-arrow-up me-1"></i>${pend.length} ação(ões) deste computador ainda não enviadas</h2>
      <ol class="list-unstyled small mb-0">${pend.map((p) => linha({
        em: p.ocorrido_em, tipo: p.tipo === 'remover' ? 'correcao' : p.tipo === 'pedra' ? 'pedra' : p.tipo === 'limpar' ? 'fim' : 'info',
        titulo: (NOME_ACAO[p.tipo] ?? p.tipo) + (p.tipo === 'pedra' ? ' ' + letra(p.dados?.numero) + p.dados?.numero : ''),
        detalhe: p.tipo === 'remover' ? letra(p.dados?.numero) + p.dados?.numero : p.dados?.cartela ?? p.dados?.codigo ?? '',
      })).join('')}</ol><div class="small text-secondary mt-2">Sincronize para elas entrarem no registro oficial.</div></div></div>`;
  }
  if (!r.online) {
    html += '<div class="alert alert-warning"><i class="fas fa-wifi me-1"></i>Sem internet: o registro completo aparece quando o computador estiver conectado.</div>';
  } else {
    const s = r.servidor as Json;
    for (const rod of s.rodadas as Json[]) {
      const ev = rod.eventos as Json[];
      html += `<details class="card mb-3" ${ev.length ? 'open' : ''}><summary class="card-body py-2 d-flex flex-wrap align-items-center gap-2" style="cursor:pointer">
        <strong class="fs-5">${esc(rod.nome)}</strong><span class="small text-secondary">${rod.pedras.length} pedra(s) no quadro agora</span>
        ${rod.quadros_limpos ? `<span class="badge text-bg-light border">${rod.quadros_limpos} quadro(s) limpo(s)</span>` : ''}${rod.pedras_removidas ? `<span class="badge text-bg-warning">${rod.pedras_removidas} pedra(s) removida(s)</span>` : ''}</summary>
        <div class="card-body pt-0">${ev.length ? `<ol class="list-unstyled small mb-0">${ev.map(linha).join('')}</ol>` : '<span class="text-secondary small">Nenhuma pedra nesta rodada ainda.</span>'}</div></details>`;
    }
    if ((s.cartelas as Json[]).length) {
      html += `<h2 class="h5 mt-4 mb-3">Cartelas alteradas durante a festa</h2><ol class="list-unstyled small">${(s.cartelas as Json[]).map(linha).join('')}</ol>`;
    }
  }
  document.getElementById('registro')!.innerHTML = html;
}

async function iniciarOuFestas() { situacao = await invoke('situacao'); situacao.logado ? telaFestas() : iniciar(); }

function desenharSorteio() {
  agendarAjuste();
  // rodadas
  document.getElementById('rodadas')!.innerHTML = estado.rodadas.map((r: Json) =>
    `<button class="nav-link ${r.id === rodadaId ? 'active' : ''}" data-r="${r.id}">${esc(r.nome)}</button>`).join('');
  document.querySelectorAll<HTMLButtonElement>('#rodadas [data-r]').forEach((b) => b.addEventListener('click', () => { rodadaId = Number(b.dataset.r); desenharSorteio(); }));

  // prêmio no telão: opcional, só informativo; clicar de novo no escolhido tira do telão
  const exibido = premioExibido();
  const premios = rodadaAtual()?.premios ?? [];
  const cardPremios = document.getElementById('card-premios')!;
  cardPremios.classList.toggle('d-none', !premios.length);
  document.body.classList.toggle('sem-premio-escolhido', !exibido);
  document.getElementById('premios')!.innerHTML = '<span class="small text-secondary me-1 so-operador"><i class="fas fa-tv me-1"></i>Prêmio no telão (opcional):</span>'
    + premios.map((p: Json) => exibido && p.id === exibido.id
      ? `<button class="btn btn-sm btn-primary rounded-pill fs-6 btn-premio is-exibido" data-id="${p.id}" title="Clique para tirar do telão"><i class="fas fa-gift me-1"></i>Sorteando: ${esc(rotulo(p))}</button>`
      : `<button class="btn btn-sm btn-outline-primary rounded-pill so-operador btn-premio" data-id="${p.id}" title="Mostrar no telão">${esc(rotulo(p))}</button>`).join('');
  document.querySelectorAll<HTMLButtonElement>('.btn-premio').forEach((b) => b.addEventListener('click', () => {
    const id = Number(b.dataset.id);
    acao('escolher', { premio_id: exibido && exibido.id === id ? null : id });
  }));

  // quadro, bola e contadores
  const ordem = pedrasOrdem();
  const saiu = new Set(ordem.map((p) => p.numero));
  document.querySelectorAll<HTMLButtonElement>('.draw-cell').forEach((c) => {
    const on = saiu.has(Number(c.dataset.n));
    c.classList.toggle('drawn', on); c.setAttribute('aria-pressed', String(on));
  });
  const ultima = ordem.slice(-1)[0];
  const bola = document.getElementById('bola')!;
  bola.className = 'bingo-ball bingo-ball--giant mx-auto mb-3' + (ultima ? ` ball-${letra(ultima.numero).toLowerCase()}` : ' is-empty');
  bola.innerHTML = ultima ? `<span class="ball-letter">${letra(ultima.numero)}</span><span class="ball-number">${ultima.numero}</span>` : '<span class="ball-number">?</span>';
  document.getElementById('st-saiu')!.textContent = String(ordem.length);
  document.getElementById('st-resta')!.textContent = String(75 - ordem.length);
  document.getElementById('pendentes')!.textContent = estado.pendentes ? `${estado.pendentes} ação(ões) para enviar` : '';

  document.getElementById('ordem')!.innerHTML = ordem.length
    ? ordem.map((p, i) => `<span class="order-chip ${i === ordem.length - 1 ? 'is-last' : ''}">${letra(p.numero)}${p.numero}</span>`).join('')
    : '<span class="text-secondary">Nenhuma pedra sorteada ainda.</span>';
  (document.getElementById('btn-remover') as HTMLButtonElement).disabled = !ordem.length || !!ultima?.automatico;
  (document.getElementById('btn-limpar') as HTMLButtonElement).disabled = !ordem.length;
  carregarRanking();
}

async function carregarRanking() {
  if (!opcao('top5')) return;
  try {
    const top: Json[] = await chamar('ranking', { festaId, rodadaId });
    document.getElementById('top5')!.innerHTML = top.length ? top.map((t) => `
      <div class="top5-chip ${t.faltam === 0 ? 'is-bateu' : t.faltam === 1 ? 'is-hot' : ''}"><span class="top5-chip-pos">${t.posicao}</span>
      <span class="top5-chip-pct">${t.percentual.toLocaleString('pt-BR', { minimumFractionDigits: 1, maximumFractionDigits: 1 })}%</span>
      <span class="top5-chip-faltam">${t.faltam === 0 ? 'BATEU!' : t.faltam === 1 ? 'falta 1' : 'faltam ' + t.faltam}</span>
      <span class="top5-chip-bar"><span style="width:${t.percentual}%"></span></span></div>`).join('')
      : '<span class="text-secondary small">O ranking aparece assim que a primeira pedra sair.</span>';
    agendarAjuste();
  } catch { /* sem festa */ }
}

async function acao(tipo: string, dados: Json = {}) {
  try { await chamar('acao', { festaId, rodadaId, tipo, dados }); await recarregar(); return true; }
  catch (e) { mostrarErro(e); return false; }
}

function falar(texto: string) {
  if (!opcao('narrar')) return;
  try { const u = new SpeechSynthesisUtterance(texto); u.lang = 'pt-BR'; u.rate = 0.9; speechSynthesis.cancel(); speechSynthesis.speak(u); } catch { /* sem voz */ }
}

async function clicarPedra(e: Event) {
  const cel = (e.target as HTMLElement).closest<HTMLButtonElement>('.draw-cell');
  if (!cel) return;
  const n = Number(cel.dataset.n);
  const pedra = pedrasOrdem().find((p) => p.numero === n);
  if (pedra) return remover(n, pedra.automatico);
  if (await acao('pedra', { numero: n })) falar(`${letra(n)}, ${n}`);
}

async function remover(n: number, automatico: boolean) {
  if (automatico) return toast('Pedra sorteada pelo sistema não pode ser removida.', 'error');
  acao('remover', { numero: n });
}

async function limpar() {
  const r = await Swal.fire({
    icon: 'question', title: 'Limpar o quadro?',
    text: `As ${pedrasOrdem().length} pedra(s) saem do quadro para começar o próximo prêmio. Elas ficam no registro do sorteio.`,
    showCancelButton: true, confirmButtonText: 'Limpar quadro', cancelButtonText: 'Voltar',
  });
  if (r.isConfirmed && await acao('limpar')) toast('Quadro limpo. Bom próximo prêmio!', 'success');
}

let sorteando = false;
async function sortear() {
  if (sorteando) return;
  sorteando = true;
  try {
    const p: Json = await chamar('sortear', { festaId, rodadaId });
    await revelar(p);
    await recarregar();
    if (p.bateu > 0) toast('Alguém bateu! Confira a cartela.', 'success');
  } catch (e) { mostrarErro(e); } finally { sorteando = false; }
}

/** Revelação com suspense (cor → letra → número) quando alguém está a uma pedra de bater. */
async function revelar(p: Json) {
  const espera = (ms: number) => new Promise((r) => setTimeout(r, matchMedia('(prefers-reduced-motion: reduce)').matches ? Math.min(ms, 400) : ms));
  const ov = document.createElement('div');
  ov.className = 'revelacao';
  ov.innerHTML = `<div><div class="revelacao-aviso">${p.suspense ? 'Atenção… tem cartela a uma pedra de bater!' : ''}</div><div class="bingo-ball is-empty mx-auto" id="rv"><span class="ball-number">?</span></div></div>`;
  document.body.appendChild(ov);
  const b = ov.querySelector('#rv') as HTMLElement;
  const cor = `ball-${p.letra.toLowerCase()}`;
  if (p.suspense) {
    falar('Atenção!');
    await espera(1700); b.className = `bingo-ball mx-auto ${cor}`;
    await espera(1500); b.innerHTML = `<span class="ball-number">${p.letra}</span>`;
    await espera(1500);
  } else { await espera(800); b.className = `bingo-ball mx-auto ${cor}`; }
  b.innerHTML = `<span class="ball-letter">${p.letra}</span><span class="ball-number">${p.numero}</span>`;
  falar(`${p.letra}, ${p.numero}`);
  if (p.bateu > 0 && typeof confetti === 'function') confetti({ particleCount: 160, spread: 90, origin: { y: 0.6 } });
  await espera(p.bateu > 0 ? 3500 : p.suspense ? 2000 : 1300);
  ov.remove();
}

async function conferir(campo = 'cod') {
  const input = document.getElementById(campo) as HTMLInputElement;
  const codigo = input.value.trim();
  if (!codigo) return toast('Digite o número ou o código da cartela.');
  try {
    const r: Json = await chamar('conferir', { festaId, rodadaId, codigo });
    const sorteados = new Set<number>(r.sorteados);
    const grade = `<table class="cartela-grade"><tr>${LETRAS.map((l) => `<th>${l}</th>`).join('')}</tr>${r.linhas.map((l: Json[]) => `<tr>${l.map((n) =>
      n === null ? '<td class="livre">LIVRE</td>' : `<td class="${sorteados.has(n) ? 'marcado' : ''}">${n}</td>`).join('')}</tr>`).join('')}</table>`;
    const status = r.cancelada ? `<h5 class="text-danger mt-3">CARTELA CANCELADA</h5><div class="small">${esc(r.motivo_cancelamento)}</div>`
      : r.bateu_na_pedra ? `<h5 class="text-success mt-3">CARTELA COMPLETA!</h5><div class="small">Bateu na ${r.bateu_na_pedra}ª pedra.</div>`
      : `<h5 class="text-danger mt-3">Ainda não bateu — faltam ${r.faltam}.</h5>`;
    // 2 verificações: o número e o código impressos na cartela de papel têm que bater com estes
    const verificar = `<div class="alert alert-info py-2 small text-start"><i class="fas fa-shield-halved me-1"></i>Confira na cartela de papel: número <strong>${esc(r.codigo)}</strong>
      e código <strong class="font-monospace">${esc(r.verificador ?? '—')}</strong>. Se um dos dois não bater, a cartela não é desta festa.</div>`;
    const bateu = !!r.bateu_na_pedra && !r.cancelada;
    const reacao = `<div class="conferir-reacao ${bateu ? 'is-bateu' : 'is-triste'}" aria-hidden="true"><i class="fas ${bateu ? 'fa-trophy' : 'fa-face-sad-tear'}"></i></div>`;
    Swal.fire({
      title: `Cartela ${esc(r.codigo)} <span class="badge text-bg-dark font-monospace fs-6 align-middle">${esc(r.verificador ?? '')}</span>`,
      html: `${verificar}<div class="small text-secondary mb-2">${r.premio ? esc(r.premio) + ' · ' : ''}${r.total_pedras} pedra(s) no quadro</div>${grade}${reacao}${status}`,
      confirmButtonText: 'Fechar', width: 580,
    });
    input.value = '';
    if (bateu) fogos();
  } catch (e) { mostrarErro(e); }
}

/** Comemoração: confete + "fogos" (estouros em pontos aleatórios por ~3 s). */
function fogos() {
  if (typeof confetti !== 'function' || matchMedia('(prefers-reduced-motion: reduce)').matches) return;
  const z = { zIndex: 2000 };
  const cores = ['#2563eb', '#e11d48', '#f59e0b', '#16a34a', '#7c3aed'];
  confetti({ ...z, particleCount: 160, spread: 80, origin: { y: 0.6 } });
  const fim = Date.now() + 3000;
  (function estouro() {
    confetti({ ...z, particleCount: 45, startVelocity: 32, spread: 360, ticks: 70, colors: cores,
      origin: { x: 0.15 + Math.random() * 0.7, y: 0.15 + Math.random() * 0.35 } });
    if (Date.now() < fim) setTimeout(estouro, 380);
  })();
}

// ------------------------------------------------------------------ telão: maior pedra que cabe na tela
/** Escolhe a maior altura de pedra (--celula-telao) que deixa tudo cabendo sem rolagem, em qualquer tela. */
let ajusteAgendado = false;
function agendarAjuste() {
  if (ajusteAgendado) return;
  ajusteAgendado = true;
  requestAnimationFrame(() => { ajusteAgendado = false; ajustarTelao(); });
}
function ajustarTelao() {
  const b = document.body;
  if (!b.classList.contains('telao')) { b.classList.remove('telao-ajustado'); b.style.removeProperty('--celula-telao'); return; }
  const cel = document.querySelector('.draw-cell');
  if (!cel) return;
  b.classList.add('telao-ajustado');
  const cabe = () => document.documentElement.scrollHeight <= window.innerHeight + 1;
  let lo = 18;
  b.style.setProperty('--celula-telao', lo + 'px');
  // pedra no máximo 1,4× mais alta que larga; em telas mais quadradas (4:3, 5:4) até 1,9×
  const proporcao = window.innerWidth / window.innerHeight < 1.5 ? 1.9 : 1.4;
  let hi = Math.min(window.innerHeight * 0.16, cel.getBoundingClientRect().width * proporcao);
  for (let i = 0; i < 9; i++) {
    const meio = (lo + hi) / 2;
    b.style.setProperty('--celula-telao', meio + 'px');
    if (cabe()) lo = meio; else hi = meio;
  }
  b.style.setProperty('--celula-telao', Math.floor(lo) + 'px');
}
window.addEventListener('resize', agendarAjuste);

async function entrarTelao() {
  document.body.classList.add('telao');
  agendarAjuste();
  document.getElementById('btn-sair-telao')?.classList.remove('d-none');
  try { await getCurrentWindow().setFullscreen(true); } catch { /* sem permissão */ }
}
async function sairTelao() {
  document.body.classList.remove('telao');
  agendarAjuste();
  if (avisoPendente) setTimeout(() => { if (avisoPendente && !document.body.classList.contains('telao')) oferecerAtualizacao(avisoPendente); }, 800);
  document.getElementById('btn-sair-telao')?.classList.add('d-none');
  try { await getCurrentWindow().setFullscreen(false); } catch { /* ok */ }
}

// ------------------------------------------------------------------ atualização automática
/*
 * Ao abrir o app (depois de sincronizar) ele pergunta ao servidor do Bingo Digital se há versão nova
 * (Admin → Versões do app). Se houver: "Atualizar agora" baixa com barra de progresso, instala e
 * reinicia; "Agora não" segue na versão atual e pergunta de novo na próxima vez que o app abrir.
 * Com o telão aberto o aviso espera o operador sair. Versão obrigatória: sem "Agora não".
 * O arquivo é conferido com a assinatura do Bingo Digital (arquivo sem a nossa assinatura é recusado).
 */
let atualizacaoEmAndamento = false;
let versaoJaOferecida = '';     // "Agora não" vale até o app fechar
let avisoPendente: Json = null; // aviso guardado enquanto o telão está aberto

async function verificarAtualizacao(forcar = false) {
  if (import.meta.env.DEV && !forcar) return;
  if (atualizacaoEmAndamento) return;
  atualizacaoEmAndamento = true;
  try {
    const nova: Json = await chamar('atualizacao_procurar');
    if (!nova) return;
    if (nova.obrigatoria || forcar) { await atualizarAgora(nova, true); return; }
    if (versaoJaOferecida === nova.versao) return;
    if (document.body.classList.contains('telao') || document.querySelector('.swal2-container')) { avisoPendente = nova; return; }
    await oferecerAtualizacao(nova);
  } catch { /* sem internet ou servidor fora: segue normal e tenta na próxima vez */ }
  finally { atualizacaoEmAndamento = false; }
}

async function oferecerAtualizacao(nova: Json) {
  versaoJaOferecida = nova.versao;
  avisoPendente = null;
  const r = await Swal.fire({
    icon: 'info', title: `Nova versão ${esc(nova.versao)} disponível`,
    html: `<p class="small mb-2">Você está na versão ${esc(nova.atual)}.</p>`
      + (nova.notas ? `<div class="small text-start border rounded p-2 mb-2" style="white-space:pre-line"><strong>O que mudou:</strong>\n${esc(nova.notas)}</div>` : '')
      + '<span class="small text-secondary">Leva poucos instantes: o app baixa, instala e abre de novo. Seus dados ficam.</span>',
    showCancelButton: true, confirmButtonText: '<i class="fas fa-download me-1"></i> Atualizar agora', cancelButtonText: 'Agora não', reverseButtons: true,
  });
  if (r.isConfirmed) await atualizarAgora(nova, false);
}

async function atualizarAgora(nova: Json, obrigatoria: boolean) {
  if (obrigatoria) {
    await Swal.fire({ icon: 'warning', title: `Atualização obrigatória: versão ${esc(nova.versao)}`,
      text: 'Esta versão é necessária para continuar sincronizando com o servidor. O sorteio já feito neste computador não se perde.',
      confirmButtonText: 'Atualizar agora', allowOutsideClick: false, allowEscapeKey: false });
  }
  ocupado.abrir(`Atualizando para a versão ${nova.versao}…`, 'Baixando…');
  const desligar = await listen<Json>('atualizacao-progresso', (ev) => {
    const { baixado, total } = ev.payload;
    ocupado.progresso(total ? pct(baixado, total) : null, total ? `Baixando… ${(baixado / 1048576).toFixed(1)} de ${(total / 1048576).toFixed(1)} MB` : 'Baixando…');
  });
  try {
    await chamar('atualizacao_baixar');
    ocupado.progresso(100, 'Instalando e reiniciando o app…');
    await chamar('atualizacao_instalar'); // reinicia o app
  } catch (e) {
    ocupado.fechar();
    await Swal.fire({ icon: 'error', title: 'Não foi possível atualizar', text: String((e as Json)?.message ?? e) + ' Tente de novo com a internet funcionando.', confirmButtonText: 'Tentar de novo' });
    atualizacaoEmAndamento = false;
    if (obrigatoria) verificarAtualizacao(true);
  } finally { desligar(); }
}

// Produção: sem menu de contexto ("Inspecionar") e sem atalhos de ferramentas de desenvolvedor.
// (O app de produção do Tauri já sai sem DevTools; isto fecha os atalhos que sobram.)
if (!import.meta.env.DEV) {
  document.addEventListener('contextmenu', (e) => e.preventDefault());
  document.addEventListener('keydown', (e) => {
    const k = e.key.toLowerCase();
    const atalho = e.key === 'F12' || ((e.ctrlKey || e.metaKey) && e.shiftKey && ['i', 'j', 'c'].includes(k)) || ((e.ctrlKey || e.metaKey) && ['u', 'r'].includes(k)) || e.key === 'F5';
    if (atalho) { e.preventDefault(); e.stopPropagation(); }
  }, true);
}

iniciar(); // com conta: telaFestas sincroniza tudo e depois procura atualização
if (!situacao?.logado) setTimeout(() => { if (!situacao?.logado) verificarAtualizacao(); }, 1500); // sem conta (demonstração)
