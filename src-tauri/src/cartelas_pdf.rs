//! Desenho dos PDFs das cartelas no computador do cliente (mesmo layout da versão web:
//! resources/views/pdf/folha_inteira.blade.php e meia_folha.blade.php).
//!
//! - Folha inteira: 1 cartela por página, uma grade por prêmio (3 por linha); com 1 prêmio só, uma
//!   grade grande e centralizada. Embaixo: [chancela | número + código + barras | logo da festa],
//!   logos dos patrocinadores (revezam entre as cartelas) e o rodapé.
//! - Meia folha (só extras): 2 cartelas por página em CORTE E EMPILHAMENTO: em cima a 1ª metade
//!   do arquivo, embaixo a 2ª; cada metade com cabeçalho, grade à esquerda e identificação à direita.
//! - Rodada extra com marca: "EXTRA 1" grande, vermelho, em pé na lateral esquerda.
//! - Rodada extra embaralhada: mesma ordem do servidor (jogo::colunas_para).

use crate::db::Cartela;
use crate::jogo;
use crate::pdf::{code39, Documento, Fonte, Pagina, A4};
use serde_json::Value;

pub struct Arte {
    pub cabecalho: Option<Vec<u8>>,
    pub marca_dagua: Option<Vec<u8>>,
    pub coringa: Option<Vec<u8>>,
    pub logo: Option<Vec<u8>>,
    /// QR code fixo do Pix (opcional; vai com o texto Layout::pix_texto).
    pub pix: Option<Vec<u8>>,
    /// QR code do selo "Cartelas geradas pelo Bingo Digital" (abre o site).
    pub selo_qr: Option<Vec<u8>>,
    /// Logos dos patrocinadores, na ordem do painel (só com layout "logos").
    pub patrocinadores: Vec<Vec<u8>>,
}

impl Arte {
    #[cfg(test)]
    pub fn vazia() -> Self {
        Arte { cabecalho: None, marca_dagua: None, coringa: None, logo: None, pix: None, selo_qr: None, patrocinadores: Vec::new() }
    }
}

/// Cabeçalho escrito (sem imagem): faixa colorida com título e subtítulo.
#[derive(Clone, Default)]
pub struct CabecalhoTexto {
    pub titulo: String,
    pub subtitulo: String,
    pub fundo: (f32, f32, f32),
    pub texto: (f32, f32, f32),
}

/// Opções do montador (Festa::layoutPdf no servidor).
#[derive(Clone, Default)]
pub struct Layout {
    pub marca_grande: bool,
    pub chancela: bool,
    pub mostrar_rodape: bool,
    /// Some = cabeçalho só escrito; None = a imagem do cabeçalho (se houver).
    pub cabecalho_texto: Option<CabecalhoTexto>,
    /// "Pix: R$ 10,00 (dez reais)" — só com o QR (Arte::pix).
    pub pix_texto: Option<String>,
    /// Selo do Bingo Digital (por plano; nas festas do administrador, normal ou grande). None = não mostra.
    pub selo: Option<Selo>,
}

#[derive(Clone, Default)]
pub struct Selo {
    pub grande: bool,
    pub texto: String,
    pub site: String,
}

/// Proporção padrão do cabeçalho (Festa::CABECALHO_PROPORCAO): caixa fixa, nunca empurra as grades.
const CABECALHO_PROPORCAO: f32 = 6.0;

/// "#1d4ed8" → (r, g, b) de 0 a 1.
pub fn cor_hex(hex: &str) -> Option<(f32, f32, f32)> {
    let h = hex.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(|v| v as f32 / 255.0);
    Some((c(0)?, c(2)?, c(4)?))
}

pub struct Contexto<'a> {
    pub festa_nome: &'a str,
    pub rodape: String,
    pub rodada: &'a Value,
    pub premios: Vec<(i64, String)>, // (ordem, título)
    /// Referência discreta impressa em cada página (festa, computador, lote): rastreia vazamento.
    pub rastreio: String,
    pub layout: Layout,
}

/// Logos por cartela (Festa::LOGOS_POR_CARTELA): passando disso, os patrocinadores se revezam.
const LOGOS_FOLHA: usize = 8; // principal, em 2 linhas: de 3 (até 6) ou de 4 (7 e 8)
const LOGOS_EXTRA: usize = 6; // extra em folha inteira: uma linha só
const LOGOS_MEIA: usize = 4;
const VERMELHO: (f32, f32, f32) = (0.84, 0.18, 0.18);

/// Mesma regra do servidor (Festa::patrocinadoresDaCartela): blocos de n cartelas (n = nº de
/// patrocinadores); no bloco, a cartela j recebe as posições (j·vagas + k) mod n — sem repetir e cada um
/// exatamente "vagas" vezes por bloco —, embaralhadas por bloco com i → (i·s + bloco) mod n (s primo com n).
pub fn patrocinadores_da_cartela(numero: i64, total: usize, vagas: usize) -> Vec<usize> {
    if total <= vagas {
        return (0..total).collect();
    }
    let mdc = |mut a: usize, mut b: usize| { while b != 0 { (a, b) = (b, a % b); } a };
    let coprimos: Vec<usize> = (1..total).filter(|s| mdc(*s, total) == 1).collect();
    let numero = numero.max(1) as usize - 1;
    let (bloco, j) = (numero / total, numero % total);
    let s = coprimos[bloco % coprimos.len()];
    (0..vagas).map(|k| (((j * vagas + k) % total) * s + bloco) % total).collect()
}

fn marca_rastreio(p: &mut Pagina, ctx: &Contexto) {
    if !ctx.rastreio.is_empty() {
        p.cor(0.6, 0.6, 0.6);
        let w = crate::pdf::largura(&ctx.rastreio, Fonte::Normal, 5.5) * PT;
        p.texto(A4.0 - 6.0 - w, A4.1 - 4.0, &ctx.rastreio, Fonte::Normal, 5.5);
        p.cor(0.0, 0.0, 0.0);
    }
}

const PT: f32 = 0.3528; // 1 pt em mm

fn cortar(texto: &str, fonte: Fonte, tamanho: f32, max_mm: f32) -> String {
    let mut t = texto.to_string();
    while !t.is_empty() && crate::pdf::largura(&t, fonte, tamanho) * PT > max_mm {
        t.pop();
    }
    if t.len() < texto.len() && t.chars().count() > 1 {
        t.pop();
        t.push('…');
    }
    t
}

struct Imgs {
    cabecalho: Option<usize>,
    marca: Option<usize>,
    coringa: Option<usize>,
    logo: Option<usize>,
    pix: Option<usize>,
    selo_qr: Option<usize>,
    patrocinadores: Vec<usize>,
}

fn registrar(doc: &mut Documento, arte: &Arte) -> Imgs {
    let mut r = |b: &Option<Vec<u8>>| b.as_ref().and_then(|b| crate::pdf::Imagem::carregar(b)).map(|i| doc.registrar_imagem(i));
    let (cabecalho, marca, coringa, logo, pix, selo_qr) = (r(&arte.cabecalho), r(&arte.marca_dagua), r(&arte.coringa), r(&arte.logo), r(&arte.pix), r(&arte.selo_qr));
    let patrocinadores = arte.patrocinadores.iter().filter_map(|b| crate::pdf::Imagem::carregar(b)).map(|i| doc.registrar_imagem(i)).collect();
    Imgs { cabecalho, marca, coringa, logo, pix, selo_qr, patrocinadores }
}

/// "1º prêmio: Moto 0 km" (se o título já começa com número, "1° prêmio 10.000", fica como está).
fn rotulo(ordem: i64, titulo: &str) -> String {
    if titulo.trim_start().starts_with(|c: char| c.is_ascii_digit()) { titulo.to_string() } else { format!("{ordem}º prêmio: {titulo}") }
}

/// Grade B-I-N-G-O. (x, y) = canto superior esquerdo; devolve a altura usada.
#[allow(clippy::too_many_arguments)]
fn grade(p: &mut Pagina, doc: &Documento, imgs: &Imgs, x: f32, y: f32, w: f32, h_cab: f32, h_cel: f32, fonte_num: f32, linhas: &[Vec<Option<i64>>]) -> f32 {
    let cw = w / 5.0;
    p.espessura(0.3);
    let fonte_cab = (fonte_num * 0.8).max(11.0);
    for (c, l) in ["B", "I", "N", "G", "O"].iter().enumerate() {
        let cx = x + cw * c as f32;
        p.cor(0.93, 0.93, 0.93).retangulo(cx, y, cw, h_cab, true, false);
        p.cor(0.0, 0.0, 0.0).retangulo(cx, y, cw, h_cab, false, true);
        p.texto_centro(cx + cw / 2.0, y + h_cab / 2.0 + fonte_cab * PT * 0.35, l, Fonte::Negrito, fonte_cab);
    }
    for (li, linha) in linhas.iter().enumerate() {
        let cy = y + h_cab + h_cel * li as f32;
        for (c, n) in linha.iter().enumerate() {
            let cx = x + cw * c as f32;
            p.cor(0.0, 0.0, 0.0).retangulo(cx, cy, cw, h_cel, false, true);
            match n {
                Some(n) => {
                    p.texto_centro(cx + cw / 2.0, cy + h_cel / 2.0 + fonte_num * PT * 0.35, &format!("{n:02}"), Fonte::Negrito, fonte_num);
                }
                None => match imgs.coringa {
                    Some(i) => {
                        p.imagem(i, doc.dimensoes(i), cx + 0.8, cy + 0.8, cw - 1.6, h_cel - 1.6, false);
                    }
                    None => {
                        p.texto_centro(cx + cw / 2.0, cy + h_cel / 2.0 + 1.0, "LIVRE", Fonte::Negrito, 6.5);
                    }
                },
            }
        }
    }
    h_cab + h_cel * linhas.len() as f32
}

/// "EXTRA 1" em pé, vermelho, numa faixa de 20 mm à esquerda (de y0 até y0 + altura).
fn lateral(p: &mut Pagina, ctx: &Contexto, y0: f32, altura: f32, tamanho: f32) {
    lateral_em(p, ctx, 19.0, y0, altura, tamanho);
}

fn lateral_em(p: &mut Pagina, ctx: &Contexto, cx: f32, y0: f32, altura: f32, tamanho: f32) {
    let nome = ctx.rodada["nome"].as_str().unwrap_or("").to_uppercase();
    p.cor(VERMELHO.0, VERMELHO.1, VERMELHO.2).texto_girado_solido(cx, y0 + altura / 2.0, &nome, Fonte::Negrito, tamanho, 90.0);
    p.cor(0.0, 0.0, 0.0);
}

/// Cabeçalho numa caixa de tamanho fixo (x, y, w, h): imagem encaixada sem distorcer ou faixa escrita.
/// Devolve a altura usada (0 = sem cabeçalho).
fn cabecalho(p: &mut Pagina, doc: &Documento, imgs: &Imgs, ctx: &Contexto, x: f32, y: f32, w: f32, h: f32) -> f32 {
    // o servidor manda o cabeçalho PRONTO como imagem (arte com zoom ou cabeçalho escrito com degradê):
    // ela tem preferência; a faixa escrita aqui é só para festas salvas antes disso
    if let Some(i) = imgs.cabecalho {
        p.imagem(i, doc.dimensoes(i), x, y, w, h, false);
        return h;
    }
    if let Some(t) = &ctx.layout.cabecalho_texto {
        if t.titulo.trim().is_empty() {
            return 0.0;
        }
        let tem_sub = !t.subtitulo.trim().is_empty();
        let (tam_titulo, tam_sub) = (h * if tem_sub { 1.05 } else { 1.3 }, h * 0.5);
        p.cor(t.fundo.0, t.fundo.1, t.fundo.2).retangulo(x, y, w, h, true, false);
        p.cor(t.texto.0, t.texto.1, t.texto.2);
        let cx = x + w / 2.0;
        if tem_sub {
            let base = y + h / 2.0 - 0.6;
            p.texto_centro(cx, base, &cortar(&t.titulo, Fonte::Negrito, tam_titulo, w - 8.0), Fonte::Negrito, tam_titulo);
            p.texto_centro(cx, base + tam_sub * PT + 2.2, &cortar(&t.subtitulo, Fonte::Normal, tam_sub, w - 8.0), Fonte::Normal, tam_sub);
        } else {
            p.texto_centro(cx, y + h / 2.0 + tam_titulo * PT * 0.35, &cortar(&t.titulo, Fonte::Negrito, tam_titulo, w - 8.0), Fonte::Negrito, tam_titulo);
        }
        p.cor(0.0, 0.0, 0.0);
        return h;
    }
    0.0
}

/// Altura da faixa do cabeçalho: a da imagem pronta (o servidor já desenha na escala escolhida, de
/// 100% a 200%), limitada entre a base e o máximo do formato (meia folha cabe menos).
fn altura_cabecalho(doc: &Documento, imgs: &Imgs, largura: f32, base: f32, maximo: f32) -> f32 {
    match imgs.cabecalho {
        Some(i) => {
            let (iw, ih) = doc.dimensoes(i);
            (largura * ih as f32 / iw.max(1) as f32).clamp(base, maximo)
        }
        None => base,
    }
}

fn tem_lateral(ctx: &Contexto) -> bool {
    ctx.rodada["tipo"] != "principal" && ctx.rodada["marca_extra"].as_bool() == Some(true)
}

/// Bloco de identificação centrado em cx: número, código e barras (menores). Devolve a altura.
fn identificacao(p: &mut Pagina, ctx: &Contexto, cartela: &Cartela, cx: f32, y: f32, com_rodada: bool) -> f32 {
    let numero = if com_rodada { format!("Cartela {} · {}", cartela.codigo, ctx.rodada["nome"].as_str().unwrap_or("")) } else { format!("Cartela {}", cartela.codigo) };
    p.cor(0.0, 0.0, 0.0).texto_centro(cx, y + 3.8, &numero, Fonte::Negrito, 10.5);
    let mut h = 5.0;
    if let Some(v) = &cartela.verificador {
        p.texto_centro(cx, y + h + 3.4, &format!("Código {v}"), Fonte::Negrito, 9.5);
        h += 5.0;
    }
    code39(p, cx, y + h, 34.0, 8.0, &cartela.codigo);
    h + 8.0
}

/// Texto centrado em cx, quebrando em palavras para caber em max_mm. Devolve a altura usada.
fn texto_quebrado(p: &mut Pagina, cx: f32, y: f32, texto: &str, tamanho: f32, max_mm: f32) -> f32 {
    let mut linhas: Vec<String> = Vec::new();
    let mut atual = String::new();
    for palavra in texto.split_whitespace() {
        let tentativa = if atual.is_empty() { palavra.to_string() } else { format!("{atual} {palavra}") };
        if crate::pdf::largura(&tentativa, Fonte::Normal, tamanho) * PT > max_mm && !atual.is_empty() {
            linhas.push(std::mem::take(&mut atual));
            atual = palavra.to_string();
        } else {
            atual = tentativa;
        }
    }
    if !atual.is_empty() {
        linhas.push(atual);
    }
    let passo = tamanho * PT * 1.2;
    p.cor(0.13, 0.13, 0.13);
    for (k, l) in linhas.iter().enumerate() {
        p.texto_centro(cx, y + passo * k as f32, l, Fonte::Normal, tamanho);
    }
    p.cor(0.0, 0.0, 0.0);
    passo * linhas.len() as f32
}

fn chancela(p: &mut Pagina, x: f32, y: f32, w: f32, h: f32) {
    p.cor(0.55, 0.55, 0.55).espessura(0.3).tracejado(true).retangulo(x, y, w, h, false, true).tracejado(false);
    p.texto_centro(x + w / 2.0, y + h / 2.0 + 1.0, "chancela", Fonte::Normal, 7.0);
    p.cor(0.0, 0.0, 0.0);
}

/// Faixa de logos dos patrocinadores desta cartela (revezamento) de x a x + w. Devolve a altura.
fn faixa_patrocinio(p: &mut Pagina, doc: &Documento, imgs: &Imgs, cartela: &Cartela, x: f32, y: f32, w: f32, vagas: usize, (lw, lh): (f32, f32)) -> f32 {
    if imgs.patrocinadores.is_empty() {
        return 0.0;
    }
    let numero = cartela.codigo.parse::<i64>().unwrap_or(1);
    let grupo = patrocinadores_da_cartela(numero, imgs.patrocinadores.len(), vagas);
    let slot = w / grupo.len() as f32;
    let lw = lw.min(slot - 2.0); // com 9 logos, cada um fica mais estreito
    for (k, idx) in grupo.iter().enumerate() {
        let i = imgs.patrocinadores[*idx];
        p.imagem(i, doc.dimensoes(i), x + slot * k as f32 + (slot - lw) / 2.0, y, lw, lh, false);
    }
    lh
}

fn pagina_folha_inteira(doc: &mut Documento, imgs: &Imgs, ctx: &Contexto, cartela: &Cartela) {
    let mut p = doc.nova_pagina();
    let extra = ctx.rodada["tipo"] != "principal";
    let com_lateral = tem_lateral(ctx);
    let (x0, w) = if com_lateral { (32.0, 168.0) } else { (10.0, 190.0) };

    // marca d'água: grande atrás das grades ou pequena no meio
    if let Some(i) = imgs.marca {
        if ctx.layout.marca_grande {
            p.imagem(i, doc.dimensoes(i), x0 + 2.0, 55.0, w - 4.0, 175.0, true);
        } else {
            p.imagem(i, doc.dimensoes(i), x0 + w / 2.0 - 23.0, A4.1 * 0.4, 46.0, 46.0, true);
        }
    }
    if com_lateral {
        lateral(&mut p, ctx, 10.0, A4.1 - 20.0, 76.0);
    }

    let mut y = 10.0;
    let alt = altura_cabecalho(doc, imgs, w, w / CABECALHO_PROPORCAO, w / CABECALHO_PROPORCAO * 2.0);
    let hc = cabecalho(&mut p, doc, imgs, ctx, x0, y, w, alt);
    if hc > 0.0 {
        y += hc + 2.5;
    }
    if extra && !com_lateral {
        p.cor(0.0, 0.0, 0.0).texto_centro(x0 + w / 2.0, y + 4.5, &ctx.rodada["nome"].as_str().unwrap_or("").to_uppercase(), Fonte::Negrito, 13.0);
        y += 8.0;
    }

    let linhas = jogo::linhas_para(cartela, ctx.rodada);
    if ctx.premios.len() <= 1 {
        // 1 prêmio só: grade grande e centralizada, como a cartela de papel
        let gw = if com_lateral { 118.0 } else { 130.0 };
        let gx = x0 + (w - gw) / 2.0;
        if let Some((ordem, titulo)) = ctx.premios.first() {
            p.cor(0.0, 0.0, 0.0).texto(gx, y + 6.0, &cortar(&rotulo(*ordem, titulo), Fonte::Negrito, 13.0, gw), Fonte::Negrito, 13.0);
        }
        y += 8.0;
        y += grade(&mut p, doc, imgs, gx, y, gw, 12.0, 15.0, 30.0, &linhas) + 2.0;
    } else {
        let linhas_premios = ctx.premios.len().div_ceil(3);
        let (h_cel, fonte) = if linhas_premios >= 3 { (7.0, 13.5) } else { (9.3, 16.5) };
        let col_w = w / 3.0;
        let card_w = col_w - 4.4;
        let passo = 1.5 + 4.6 + 7.0 + h_cel * 5.0 + 3.0;
        for (i, (ordem, titulo)) in ctx.premios.iter().enumerate() {
            let (col, lin) = (i % 3, i / 3);
            let x = x0 + col_w * col as f32 + 2.2;
            let yc = y + lin as f32 * passo + 1.5;
            p.cor(0.0, 0.0, 0.0).texto(x, yc + 3.3, &cortar(&rotulo(*ordem, titulo), Fonte::Negrito, 9.5, card_w), Fonte::Negrito, 9.5);
            grade(&mut p, doc, imgs, x, yc + 4.6, card_w, 7.0, h_cel, fonte, &linhas);
        }
        y += linhas_premios as f32 * passo;
    }

    // com chancela: [Pix | logo | número | chancela]; sem chancela: [Pix | número no centro | logo no canto].
    // Com o "EXTRA 1" na lateral a folha é mais estreita: colunas menores e o número sem " · Extra 1".
    let slot = if com_lateral { 40.0 } else { 46.0 };
    let com_rodada = extra && !com_lateral;
    y += 2.0;
    let mut xl = x0 + 2.0;
    if let (Some(i), Some(texto)) = (imgs.pix, &ctx.layout.pix_texto) {
        p.imagem(i, doc.dimensoes(i), xl + (slot - 2.0) / 2.0 - 10.0, y, 20.0, 20.0, false);
        texto_quebrado(&mut p, xl + (slot - 2.0) / 2.0, y + 22.5, texto, 6.3, slot - 2.0);
        xl += slot;
    }
    if ctx.layout.chancela {
        if let Some(i) = imgs.logo {
            p.imagem(i, doc.dimensoes(i), xl, y, slot - 4.0, 24.0, false);
        }
        let (esq, dir) = (xl + slot - 2.0, x0 + w - slot);
        identificacao(&mut p, ctx, cartela, (esq + dir) / 2.0, y, com_rodada);
        chancela(&mut p, x0 + w - slot + 2.0, y + 1.0, slot - 4.0, 22.0);
    } else {
        identificacao(&mut p, ctx, cartela, x0 + w / 2.0, y, com_rodada);
        if let Some(i) = imgs.logo {
            p.imagem(i, doc.dimensoes(i), x0 + w - slot + 2.0, y, slot - 4.0, 24.0, false);
        }
    }

    // patrocinadores e rodapé presos no fim da página (a sobra fica entre as grades e eles)
    let fim = A4.1 - 10.0;
    let mut yb = fim - if ctx.layout.mostrar_rodape { 6.0 } else { 0.0 };
    if !imgs.patrocinadores.is_empty() {
        let numero = cartela.codigo.parse::<i64>().unwrap_or(1);
        let vagas = if extra { LOGOS_EXTRA } else { LOGOS_FOLHA };
        let grupo = patrocinadores_da_cartela(numero, imgs.patrocinadores.len(), vagas);
        // extra: uma linha só; principal: até 3 numa linha, 4 a 6 em linhas de 3, 7 ou 8 em linhas de 4
        let por_linha = if extra || grupo.len() <= 3 { grupo.len().max(1) } else if grupo.len() <= 6 { 3 } else { 4 };
        let linhas: Vec<&[usize]> = grupo.chunks(por_linha).collect();
        yb -= 13.0 * linhas.len() as f32;
        p.cor(0.47, 0.47, 0.47).texto_centro(x0 + w / 2.0, yb - 1.5, "PATROCÍNIO", Fonte::Normal, 6.5);
        let slot = w / por_linha as f32;
        for (l, linha) in linhas.iter().enumerate() {
            for (k, idx) in linha.iter().enumerate() {
                let i = imgs.patrocinadores[*idx];
                p.imagem(i, doc.dimensoes(i), x0 + slot * k as f32 + 1.5, yb + 13.0 * l as f32 + 0.5, slot - 3.0, 11.5, false);
            }
        }
        yb -= 4.0;
    }
    // selo "Cartelas geradas pelo Bingo Digital" acima dos patrocinadores
    if let (Some(selo), Some(qr)) = (&ctx.layout.selo, imgs.selo_qr) {
        let (lado, t1, t2) = if selo.grande { (22.0, 12.0, 10.0) } else { (12.0, 7.5, 7.0) };
        let sub = if selo.grande { format!("Faça o bingo da sua festa também: {}", selo.site) } else { selo.site.clone() };
        let largura_texto = (crate::pdf::largura(&selo.texto, Fonte::Negrito, t1).max(crate::pdf::largura(&sub, Fonte::Normal, t2))) * PT;
        let total = lado + 2.0 + largura_texto;
        let xs = x0 + (w - total) / 2.0;
        yb -= lado + 1.5;
        p.imagem(qr, doc.dimensoes(qr), xs, yb, lado, lado, false);
        p.cor(0.30, 0.11, 0.58).texto(xs + lado + 2.0, yb + lado / 2.0 - 0.5, &selo.texto, Fonte::Negrito, t1);
        p.cor(0.27, 0.27, 0.27).texto(xs + lado + 2.0, yb + lado / 2.0 + t2 * PT + 0.8, &sub, Fonte::Normal, t2);
        p.cor(0.0, 0.0, 0.0);
    }
    if ctx.layout.mostrar_rodape {
        p.cor(0.2, 0.2, 0.2).texto_centro(x0 + w / 2.0, fim - 1.5, &cortar(&ctx.rodape, Fonte::Normal, 8.5, w - 4.0), Fonte::Normal, 8.5);
        p.cor(0.0, 0.0, 0.0);
    }
    marca_rastreio(&mut p, ctx);
    doc.adicionar(p);
}

fn metade(p: &mut Pagina, doc: &Documento, imgs: &Imgs, ctx: &Contexto, cartela: &Cartela, y0: f32) {
    let com_lateral = tem_lateral(ctx);
    let (x0, w) = if com_lateral { (45.0, 155.0) } else { (10.0, 190.0) };
    if let Some(i) = imgs.marca {
        if ctx.layout.marca_grande {
            p.imagem(i, doc.dimensoes(i), x0 + 4.0, y0 + 54.0, 72.0, 72.0, true); // atrás da grade, abaixo do cabeçalho
        } else {
            p.imagem(i, doc.dimensoes(i), x0 + 26.0, y0 + 60.0, 30.0, 30.0, true);
        }
    }
    if com_lateral {
        lateral_em(p, ctx, 26.0, y0 + 6.0, A4.1 / 2.0 - 12.0, 44.0); // mais para dentro da folha
    }

    let mut y = y0 + 8.0;
    // topo: cabeçalho alinhado à esquerda (com a grade) e o QR do Pix no canto direito
    let tem_pix = imgs.pix.is_some() && ctx.layout.pix_texto.is_some();
    let col_pix = if tem_pix { 40.0 } else { 0.0 };
    if let (true, Some(q), Some(texto)) = (tem_pix, imgs.pix, &ctx.layout.pix_texto) {
        let cx = x0 + w - col_pix / 2.0;
        p.imagem(q, doc.dimensoes(q), cx - 11.0, y, 22.0, 22.0, false);
        texto_quebrado(p, cx, y + 25.0, texto, 6.4, col_pix - 2.0);
    }
    let hc_larg = w - col_pix;
    let max_cab = if ctx.layout.selo.is_some() && imgs.selo_qr.is_some() { 36.0 } else { 40.0 };
    let hc_alt = altura_cabecalho(doc, imgs, w, (w / CABECALHO_PROPORCAO).min(max_cab), max_cab);
    // largura real da imagem nessa altura (para encostar à esquerda, sem sobra antes dela)
    let largura_img = imgs.cabecalho.map(|i| { let (iw, ih) = doc.dimensoes(i); (hc_alt * iw as f32 / ih.max(1) as f32).min(hc_larg) }).unwrap_or(hc_larg);
    let hc = cabecalho(p, doc, imgs, ctx, x0, y, largura_img, hc_alt);
    if hc > 0.0 {
        y += hc.max(if tem_pix { 30.0 } else { 0.0 }) + 1.5;
    } else if !com_lateral {
        p.cor(0.0, 0.0, 0.0).texto_centro(x0 + w / 2.0, y + 4.5, &ctx.rodada["nome"].as_str().unwrap_or("").to_uppercase(), Fonte::Negrito, 12.0);
        y += 7.0;
    }
    // etiqueta da rodada (deixa claro que é a extra) + prêmio(s) + faixa de cartelas
    let premios = if ctx.premios.len() == 1 {
        rotulo(ctx.premios[0].0, &ctx.premios[0].1)
    } else {
        format!("Prêmios: {}", ctx.premios.iter().map(|(o, t)| if t.trim_start().starts_with(|c: char| c.is_ascii_digit()) { t.clone() } else { format!("{o}º {t}") }).collect::<Vec<_>>().join(" · "))
    };
    let nome = ctx.rodada["nome"].as_str().unwrap_or("").to_uppercase();
    let tag_w = crate::pdf::largura(&nome, Fonte::Negrito, 8.0) * PT + 4.0;
    p.cor(VERMELHO.0, VERMELHO.1, VERMELHO.2).retangulo(x0, y + 1.0, tag_w, 4.6, true, false);
    p.cor(1.0, 1.0, 1.0).texto(x0 + 2.0, y + 4.4, &nome, Fonte::Negrito, 8.0);
    let premios = cortar(&premios, Fonte::Negrito, 9.5, (w - tag_w - 3.0).max(20.0));
    p.cor(0.0, 0.0, 0.0).texto(x0 + tag_w + 2.0, y + 4.4, &premios, Fonte::Negrito, 9.5);
    y += 8.0;

    // 3 colunas alinhadas: grade | identificação | chancela (lateral direita)
    let gw = 77.5;
    let h = grade(p, doc, imgs, x0, y, gw, 7.5, 9.6, 19.0, &jogo::linhas_para(cartela, ctx.rodada));
    let col_chancela = if ctx.layout.chancela { 26.0 } else { 0.0 };
    if ctx.layout.chancela {
        chancela(p, x0 + w - 24.0, y, 24.0, h);
    }
    let cx = x0 + gw + (w - gw - col_chancela) / 2.0;
    // número, código e barras alinhados ao topo da grade; a logo embaixo
    let mut yd = y + 2.0;
    yd += identificacao(p, ctx, cartela, cx, yd, false) + 4.0;
    if let Some(i) = imgs.logo {
        p.imagem(i, doc.dimensoes(i), cx - 16.0, yd, 32.0, 18.0, false);
    }

    // patrocinadores e rodapé presos no fim da metade
    let fim = y0 + A4.1 / 2.0 - 5.0;
    let linha_rodape = match &ctx.layout.selo { Some(_) if imgs.selo_qr.is_some() => 9.0, _ => if ctx.layout.mostrar_rodape { 4.5 } else { 0.0 } };
    let mut yb = fim - linha_rodape;
    if !imgs.patrocinadores.is_empty() {
        yb -= 10.5;
        faixa_patrocinio(p, doc, imgs, cartela, 10.0, yb, 190.0, LOGOS_MEIA, (30.0, 9.5));
    }
    if let (Some(selo), Some(qr)) = (&ctx.layout.selo, imgs.selo_qr) {
        // selo compacto na linha do rodapé (meia folha tem pouco espaço)
        let (lado, t1) = if selo.grande { (8.0, 7.3) } else { (8.0, 6.3) }; // compacto na meia folha
        p.imagem(qr, doc.dimensoes(qr), 10.0, fim - lado, lado, lado, false);
        p.cor(0.30, 0.11, 0.58).texto(10.0 + lado + 1.5, fim - lado / 2.0 - 0.3, &selo.texto, Fonte::Negrito, t1);
        p.cor(0.27, 0.27, 0.27).texto(10.0 + lado + 1.5, fim - lado / 2.0 + t1 * PT + 0.6, &selo.site, Fonte::Normal, t1 - 0.3);
        if ctx.layout.mostrar_rodape {
            let r = cortar(&ctx.rodape, Fonte::Normal, 7.5, 100.0);
            let rw = crate::pdf::largura(&r, Fonte::Normal, 7.5) * PT;
            p.cor(0.27, 0.27, 0.27).texto(200.0 - rw, fim - 1.0, &r, Fonte::Normal, 7.5);
        }
        p.cor(0.0, 0.0, 0.0);
    } else if ctx.layout.mostrar_rodape {
        p.cor(0.27, 0.27, 0.27).texto_centro(x0 + w / 2.0, fim - 1.0, &cortar(&ctx.rodape, Fonte::Normal, 7.5, w - 4.0), Fonte::Normal, 7.5);
        p.cor(0.0, 0.0, 0.0);
    }
}

/// Gera o PDF de um arquivo (lote). Devolve (bytes, páginas, detalhe do corte para meia folha).
pub fn gerar(ctx: &Contexto, arte: &Arte, cartelas: &[Cartela]) -> (Vec<u8>, usize, Option<String>) {
    let mut doc = Documento::novo();
    let imgs = registrar(&mut doc, arte);
    let meia = ctx.rodada["formato"] == "meia_folha" && ctx.rodada["tipo"] != "principal";
    let mut detalhe = None;

    if meia {
        let m = cartelas.len().div_ceil(2);
        let (cima, baixo) = cartelas.split_at(m);
        for (i, c) in cima.iter().enumerate() {
            let mut p = doc.nova_pagina();
            metade(&mut p, &doc, &imgs, ctx, c, 0.0);
            p.cor(0.45, 0.45, 0.45).espessura(0.2).tracejado(true).linha(5.0, A4.1 / 2.0, A4.0 - 5.0, A4.1 / 2.0).tracejado(false);
            p.texto(6.0, A4.1 / 2.0 - 1.2, "- - corte aqui - -", Fonte::Normal, 6.5);
            if let Some(b) = baixo.get(i) {
                metade(&mut p, &doc, &imgs, ctx, b, A4.1 / 2.0);
            }
            marca_rastreio(&mut p, ctx);
            doc.adicionar(p);
        }
        let faixa = |s: &[Cartela]| match (s.first(), s.last()) { (Some(a), Some(b)) => format!("{}–{}", a.codigo, b.codigo), _ => "—".into() };
        detalhe = Some(format!("cima {} · baixo {}", faixa(cima), faixa(baixo)));
    } else {
        for c in cartelas {
            pagina_folha_inteira(&mut doc, &imgs, ctx, c);
        }
    }

    let paginas = doc.total_paginas();
    (doc.gerar(&format!("Cartelas — {} — {}", ctx.festa_nome, ctx.rastreio)), paginas, detalhe)
}

#[cfg(test)]
mod testes {
    use super::*;
    use serde_json::json;

    #[test]
    fn patrocinadores_se_revezam_igual_ao_servidor() {
        assert_eq!(patrocinadores_da_cartela(7, 3, 9), vec![0, 1, 2]); // cabem todos
        // mesmos grupos que o servidor (Festa::patrocinadoresDaCartela) para 12 patrocinadores, 9 por folha
        assert_eq!(patrocinadores_da_cartela(1, 12, 9), vec![0, 1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(patrocinadores_da_cartela(2, 12, 9), vec![9, 10, 11, 0, 1, 2, 3, 4, 5]);
        assert_eq!(patrocinadores_da_cartela(13, 12, 9), vec![1, 6, 11, 4, 9, 2, 7, 0, 5]);
        assert_eq!(patrocinadores_da_cartela(14, 12, 9), vec![10, 3, 8, 1, 6, 11, 4, 9, 2]);
        for (total, vagas) in [(12, 9), (15, 9), (7, 4), (20, 9)] {
            let mut vezes = vec![0; total];
            for n in 1..=(total as i64 * 50) {
                let g = patrocinadores_da_cartela(n, total, vagas);
                let mut unicos = g.clone();
                unicos.sort();
                unicos.dedup();
                assert_eq!(unicos.len(), vagas, "repetiu na cartela {n}");
                for i in g {
                    vezes[i] += 1;
                }
            }
            assert!(vezes.iter().all(|v| *v == vagas * 50), "{total}/{vagas}: {vezes:?}");
        }
    }

    #[test]
    fn folha_inteira_uma_pagina_por_cartela_e_meia_folha_metade() {
        let cartelas = jogo::gerar_cartelas_demo(10);
        let arte = Arte::vazia();
        let principal = json!({ "id": 1, "nome": "Principal", "tipo": "principal", "formato": "folha" });
        let ctx = Contexto { festa_nome: "Festa", rodape: "Paróquia São Benedito — 30/11".into(), rodada: &principal,
            premios: vec![(1, "Moto".into()), (2, "TV 50\"".into()), (3, "Bicicleta".into()), (4, "Air fryer".into()), (5, "Cesta".into())], rastreio: "ref 1-7-0001".into(),
            layout: Layout { marca_grande: true, chancela: true, mostrar_rodape: true, cabecalho_texto: None, pix_texto: None, selo: None } };
        let (pdf, paginas, det) = gerar(&ctx, &arte, &cartelas);
        assert_eq!(paginas, 10);
        assert!(det.is_none());
        assert!(pdf.len() < 60_000, "10 cartelas com 5 prêmios: {} bytes", pdf.len());

        let extra = json!({ "id": 2, "nome": "Extra 1", "tipo": "extra", "formato": "meia_folha", "embaralhar": true, "semente": "x", "marca_extra": true });
        let ctx2 = Contexto { rodada: &extra, ..ctx };
        let (_, paginas, det) = gerar(&ctx2, &arte, &cartelas);
        assert_eq!(paginas, 5);
        assert_eq!(det.unwrap(), "cima 0001–0005 · baixo 0006–0010");
    }
}

#[cfg(test)]
mod amostra {
    use super::*;
    use serde_json::json;

    /// Gera amostras para conferir a olho:
    /// AMOSTRA_PDF=/pasta [CORINGA= LOGO= MARCA= PATROCINADORES=a.png,b.jpg] cargo test --lib amostra -- --ignored
    #[test]
    #[ignore]
    fn gerar_amostras() {
        let pasta = std::env::var("AMOSTRA_PDF").unwrap();
        let ler = |v: &str| std::env::var(v).ok().and_then(|c| std::fs::read(c).ok());
        let patrocinadores = std::env::var("PATROCINADORES").ok().map(|l| l.split(',').filter_map(|c| std::fs::read(c).ok()).collect()).unwrap_or_default();
        let arte = Arte { cabecalho: ler("CABECALHO"), marca_dagua: ler("MARCA"), coringa: ler("CORINGA"), logo: ler("LOGO"), pix: ler("PIX"), selo_qr: ler("SELO"), patrocinadores };
        let cartelas = jogo::gerar_cartelas_demo(500);
        let principal = json!({ "id": 1, "nome": "Principal", "tipo": "principal", "formato": "folha" });
        let premios = vec![(1, "1° premio 10.000".into()), (2, "2° premio 9.000".into()), (3, "3º prêmio - Bicicleta".into()), (4, "4° premio 1.6000".into()), (5, "5º prêmio - Air fryer".into())];
        let layout = Layout { marca_grande: true, chancela: false, mostrar_rodape: true, cabecalho_texto: Some(CabecalhoTexto {
            titulo: "Festa de São Benedito 2027".into(), subtitulo: "Paróquia São Benedito · 12 de julho · 17h".into(),
            fundo: cor_hex("#1d4ed8").unwrap(), texto: (1.0, 1.0, 1.0) }), pix_texto: Some("Pix: R$ 10,00 (dez reais)".into()),
            selo: Some(Selo { grande: std::env::var("SELO_GRANDE").is_ok(), texto: "Cartelas geradas pelo Bingo Digital".into(), site: "bingodigital.com.br".into() }) };
        let ctx = Contexto { festa_nome: "festa test", rodape: "Paróquia São Benedito — Ubatuba/SP".into(), rodada: &principal, premios, rastreio: "ref 3-7-0001".into(), layout };
        let t = std::time::Instant::now();
        let (pdf, pag, _) = gerar(&ctx, &arte, &cartelas);
        println!("principal: {pag} páginas, {} KB em {:?}", pdf.len() / 1024, t.elapsed());
        std::fs::write(format!("{pasta}/desk_principal.pdf"), pdf).unwrap();

        let extra = json!({ "id": 2, "nome": "Extra 1", "tipo": "extra", "formato": "folha", "marca_extra": true });
        let ctx2 = Contexto { rodada: &extra, premios: vec![(1, "premio rodada Extra R$ 1.000".into())], layout: ctx.layout.clone(), ..ctx };
        let (pdf, _, _) = gerar(&ctx2, &arte, &cartelas[..4]);
        std::fs::write(format!("{pasta}/desk_extra.pdf"), pdf).unwrap();

        let meia = json!({ "id": 3, "nome": "Extra 2", "tipo": "extra", "formato": "meia_folha", "embaralhar": true, "semente": "abc", "marca_extra": true });
        let ctx3 = Contexto { rodada: &meia, layout: ctx2.layout.clone(), ..ctx2 };
        let t = std::time::Instant::now();
        let (pdf, pag, det) = gerar(&ctx3, &arte, &cartelas);
        println!("extra meia folha: {pag} páginas, {} KB em {:?} — {det:?}", pdf.len() / 1024, t.elapsed());
        std::fs::write(format!("{pasta}/desk_meia.pdf"), pdf).unwrap();
    }
}
