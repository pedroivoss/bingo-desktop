//! Escritor de PDF enxuto, feito para as cartelas: só o que precisamos e nada mais.
//!
//! - Fontes Helvetica e Helvetica-Bold "padrão 14": todo leitor de PDF já tem, então NÃO vão
//!   dentro do arquivo (é o que deixa o PDF leve). Codificação WinAnsi: acentos do português ok.
//! - Conteúdo de cada página compactado (deflate). Imagens: JPEG entra como está (sem
//!   recompactar, sem perder qualidade); PNG é descompactado e recompactado sem perda.
//! - Determinístico: os mesmos dados geram o mesmo arquivo (sem data/hora dentro), então dá
//!   para conferir pelo hash se um PDF é o original.

use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::Write;

pub const MM: f32 = 72.0 / 25.4;
pub const A4: (f32, f32) = (210.0, 297.0);

#[derive(Clone, Copy)]
pub enum Fonte {
    Normal,
    Negrito,
}

/// Larguras da Helvetica (AFM, em milésimos do tamanho) para ASCII 32..126.
const LARG_NORMAL: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556,
    556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778,
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556,
    556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];
const LARG_NEGRITO: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556,
    556, 556, 333, 333, 584, 584, 584, 611, 975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778,
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 333, 278, 333, 584, 556, 333, 556, 611, 556, 611, 556, 333, 611,
    611, 278, 278, 556, 278, 889, 611, 611, 611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584,
];

/// Letra base para medir caracteres acentuados (mesma largura na Helvetica).
fn base(c: char) -> char {
    match c {
        'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
        'Á' | 'À' | 'Â' | 'Ã' | 'Ä' => 'A',
        'é' | 'ê' | 'è' => 'e',
        'É' | 'Ê' | 'È' => 'E',
        'í' | 'ì' | 'î' => 'i',
        'Í' | 'Ì' | 'Î' => 'I',
        'ó' | 'ô' | 'õ' | 'ò' | 'ö' => 'o',
        'Ó' | 'Ô' | 'Õ' | 'Ò' | 'Ö' => 'O',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'Ú' | 'Ù' | 'Û' | 'Ü' => 'U',
        'ç' => 'c',
        'Ç' => 'C',
        'ñ' => 'n',
        'Ñ' => 'N',
        _ => c,
    }
}

pub fn largura(texto: &str, fonte: Fonte, tamanho: f32) -> f32 {
    let tabela = match fonte {
        Fonte::Normal => &LARG_NORMAL,
        Fonte::Negrito => &LARG_NEGRITO,
    };
    let milesimos: u32 = texto
        .chars()
        .map(|c| match c {
            'º' | 'ª' => 365,
            '°' => 400,
            '·' | '•' => 350,
            '–' => 556,
            '—' => 1000,
            _ => {
                let b = base(c) as u32;
                if (32..=126).contains(&b) { tabela[(b - 32) as usize] as u32 } else { 556 }
            }
        })
        .sum();
    milesimos as f32 / 1000.0 * tamanho
}

/// Texto -> bytes WinAnsi (Windows-1252), escapados para string PDF.
fn win_ansi(texto: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(texto.len());
    for c in texto.chars() {
        let b: u8 = match c {
            '€' => 0x80, '‚' => 0x82, '„' => 0x84, '…' => 0x85, '•' => 0x95, '–' => 0x96, '—' => 0x97,
            '‘' => 0x91, '’' => 0x92, '“' => 0x93, '”' => 0x94,
            c if (c as u32) < 0x100 => c as u8,
            _ => b'?',
        };
        if matches!(b, b'(' | b')' | b'\\') {
            out.push(b'\\');
        }
        out.push(b);
    }
    out
}

pub struct Imagem {
    pub largura: u32,
    pub altura: u32,
    dados: Vec<u8>,
    filtro: &'static str,
    cores: &'static str,
    bits: u8,
    mascara: Option<Vec<u8>>, // alfa (PNG), já compactado
}

impl Imagem {
    /// Carrega JPEG (sem reprocessar) ou PNG (com transparência). None se não reconhecer.
    pub fn carregar(bytes: &[u8]) -> Option<Imagem> {
        if bytes.starts_with(&[0xFF, 0xD8]) {
            let (w, h, comps) = dimensoes_jpeg(bytes)?;
            return Some(Imagem {
                largura: w, altura: h, dados: bytes.to_vec(), filtro: "DCTDecode",
                cores: if comps == 1 { "DeviceGray" } else if comps == 4 { "DeviceCMYK" } else { "DeviceRGB" },
                bits: 8, mascara: None,
            });
        }
        let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
        dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut leitor = dec.read_info().ok()?;
        let mut buf = vec![0; leitor.output_buffer_size()];
        let info = leitor.next_frame(&mut buf).ok()?;
        let px = &buf[..info.buffer_size()];
        let (rgb, alfa): (Vec<u8>, Option<Vec<u8>>) = match info.color_type {
            png::ColorType::Rgba => (px.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect(), Some(px.chunks(4).map(|p| p[3]).collect())),
            png::ColorType::GrayscaleAlpha => (px.chunks(2).flat_map(|p| [p[0], p[0], p[0]]).collect(), Some(px.chunks(2).map(|p| p[1]).collect())),
            png::ColorType::Grayscale => (px.iter().flat_map(|g| [*g, *g, *g]).collect(), None),
            _ => (px.to_vec(), None),
        };
        Some(Imagem {
            largura: info.width, altura: info.height, dados: comprimir(&rgb), filtro: "FlateDecode",
            cores: "DeviceRGB", bits: 8, mascara: alfa.map(|a| comprimir(&a)),
        })
    }
}

fn dimensoes_jpeg(b: &[u8]) -> Option<(u32, u32, u8)> {
    let mut i = 2;
    while i + 9 < b.len() {
        if b[i] != 0xFF {
            i += 1;
            continue;
        }
        let marca = b[i + 1];
        let tam = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        if matches!(marca, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
            let h = u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32;
            let w = u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32;
            return Some((w, h, b[i + 9]));
        }
        i += 2 + tam;
    }
    None
}

fn comprimir(dados: &[u8]) -> Vec<u8> {
    let mut z = ZlibEncoder::new(Vec::new(), Compression::best());
    z.write_all(dados).unwrap();
    z.finish().unwrap()
}

/// Uma página: comandos de desenho em coordenadas de milímetros, origem no canto SUPERIOR esquerdo.
pub struct Pagina {
    ops: String,
    altura_mm: f32,
}

impl Pagina {
    fn y(&self, y_mm: f32) -> f32 {
        (self.altura_mm - y_mm) * MM
    }

    pub fn cor(&mut self, r: f32, g: f32, b: f32) -> &mut Self {
        self.ops += &format!("{r:.3} {g:.3} {b:.3} rg {r:.3} {g:.3} {b:.3} RG\n");
        self
    }

    pub fn espessura(&mut self, mm: f32) -> &mut Self {
        self.ops += &format!("{:.2} w\n", mm * MM);
        self
    }

    pub fn tracejado(&mut self, ligado: bool) -> &mut Self {
        self.ops += if ligado { "[4 3] 0 d\n" } else { "[] 0 d\n" };
        self
    }

    /// Retângulo (x, y = canto superior esquerdo). preencher / contornar.
    pub fn retangulo(&mut self, x: f32, y: f32, w: f32, h: f32, preencher: bool, contornar: bool) -> &mut Self {
        let op = match (preencher, contornar) { (true, true) => "B", (true, false) => "f", _ => "S" };
        self.ops += &format!("{:.2} {:.2} {:.2} {:.2} re {op}\n", x * MM, self.y(y + h), w * MM, h * MM);
        self
    }

    pub fn linha(&mut self, x1: f32, y1: f32, x2: f32, y2: f32) -> &mut Self {
        self.ops += &format!("{:.2} {:.2} m {:.2} {:.2} l S\n", x1 * MM, self.y(y1), x2 * MM, self.y(y2));
        self
    }

    /// Texto com a linha de base em y (mm).
    pub fn texto(&mut self, x: f32, y: f32, texto: &str, fonte: Fonte, tamanho: f32) -> &mut Self {
        let f = match fonte { Fonte::Normal => "F1", Fonte::Negrito => "F2" };
        self.ops += &format!("BT /{f} {tamanho:.2} Tf {:.2} {:.2} Td (", x * MM, self.y(y));
        self.string_pdf(texto);
        self.ops += ") Tj ET\n";
        self
    }

    /// Bytes WinAnsi não são UTF-8: o que não é ASCII visível vai como \ooo (octal).
    fn string_pdf(&mut self, texto: &str) {
        for b in win_ansi(texto) {
            if b.is_ascii_graphic() || b == b' ' {
                self.ops.push(b as char);
            } else {
                self.ops += &format!("\\{b:03o}");
            }
        }
    }

    pub fn texto_centro(&mut self, cx: f32, y: f32, texto: &str, fonte: Fonte, tamanho: f32) -> &mut Self {
        let w = largura(texto, fonte, tamanho) / MM;
        self.texto(cx - w / 2.0, y, texto, fonte, tamanho)
    }

    /// Texto girado OPACO, na cor atual ("EXTRA 1" em pé na lateral: graus = 90 lê de baixo para cima).
    pub fn texto_girado_solido(&mut self, cx: f32, cy: f32, texto: &str, fonte: Fonte, tamanho: f32, graus: f32) -> &mut Self {
        let (s, c) = graus.to_radians().sin_cos();
        let w = largura(texto, fonte, tamanho);
        // centro do texto em (cx, cy): recua meia largura ao longo da direção e ~1/3 da altura na perpendicular
        let alt = tamanho * 0.35;
        let (x, y) = (cx * MM - c * w / 2.0 + s * alt, self.y(cy) - s * w / 2.0 - c * alt);
        let f = match fonte { Fonte::Normal => "F1", Fonte::Negrito => "F2" };
        self.ops += &format!("q BT /{f} {tamanho:.2} Tf {c:.4} {s:.4} {:.4} {c:.4} {x:.2} {y:.2} Tm (", -s);
        self.string_pdf(texto);
        self.ops += ") Tj ET Q\n";
        self
    }

    /// Imagem já registrada no documento (índice), ajustada dentro da caixa mantendo a proporção.
    pub fn imagem(&mut self, img: usize, dims: (u32, u32), x: f32, y: f32, w: f32, h: f32, transparente: bool) -> &mut Self {
        let prop = dims.0 as f32 / dims.1.max(1) as f32;
        let (mut iw, mut ih) = (w, w / prop);
        if ih > h {
            ih = h;
            iw = h * prop;
        }
        let (ix, iy) = (x + (w - iw) / 2.0, y + (h - ih) / 2.0);
        let gs = if transparente { "/GS2 gs " } else { "" };
        self.ops += &format!("q {gs}{:.2} 0 0 {:.2} {:.2} {:.2} cm /Im{img} Do Q\n", iw * MM, ih * MM, ix * MM, self.y(iy + ih));
        self
    }
}

pub struct Documento {
    paginas: Vec<Vec<u8>>, // conteúdo já compactado
    imagens: Vec<Imagem>,
}

impl Documento {
    pub fn novo() -> Self {
        Documento { paginas: Vec::new(), imagens: Vec::new() }
    }

    pub fn registrar_imagem(&mut self, img: Imagem) -> usize {
        self.imagens.push(img);
        self.imagens.len() - 1
    }

    pub fn dimensoes(&self, img: usize) -> (u32, u32) {
        (self.imagens[img].largura, self.imagens[img].altura)
    }

    pub fn nova_pagina(&self) -> Pagina {
        Pagina { ops: String::new(), altura_mm: A4.1 }
    }

    pub fn adicionar(&mut self, p: Pagina) {
        self.paginas.push(comprimir(p.ops.as_bytes()));
    }

    pub fn total_paginas(&self) -> usize {
        self.paginas.len()
    }

    /// Monta o arquivo PDF.
    pub fn gerar(&self, titulo: &str) -> Vec<u8> {
        let mut objs: Vec<Vec<u8>> = Vec::new(); // objeto n = índice + 1
        let novo = |objs: &mut Vec<Vec<u8>>, conteudo: Vec<u8>| -> usize {
            objs.push(conteudo);
            objs.len()
        };

        let catalogo = novo(&mut objs, vec![]);
        let paginas_id = novo(&mut objs, vec![]);
        let f1 = novo(&mut objs, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec());
        let f2 = novo(&mut objs, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>".to_vec());
        let gs1 = novo(&mut objs, b"<< /Type /ExtGState /ca 0.07 /CA 0.07 >>".to_vec());
        let gs2 = novo(&mut objs, b"<< /Type /ExtGState /ca 0.10 /CA 0.10 >>".to_vec());

        let mut xobjs = String::new();
        for (i, img) in self.imagens.iter().enumerate() {
            let smask = img.mascara.as_ref().map(|m| {
                let mut o = format!(
                    "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
                    img.largura, img.altura, m.len()
                ).into_bytes();
                o.extend_from_slice(m);
                o.extend_from_slice(b"\nendstream");
                novo(&mut objs, o)
            });
            let mut o = format!(
                "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /{} /BitsPerComponent {} /Filter /{} /Length {}{} >>\nstream\n",
                img.largura, img.altura, img.cores, img.bits, img.filtro, img.dados.len(),
                smask.map(|s| format!(" /SMask {s} 0 R")).unwrap_or_default()
            ).into_bytes();
            o.extend_from_slice(&img.dados);
            o.extend_from_slice(b"\nendstream");
            let id = novo(&mut objs, o);
            xobjs += &format!("/Im{i} {id} 0 R ");
        }

        let recursos = novo(&mut objs, format!(
            "<< /Font << /F1 {f1} 0 R /F2 {f2} 0 R >> /ExtGState << /GS1 {gs1} 0 R /GS2 {gs2} 0 R >> /XObject << {xobjs}>> >>"
        ).into_bytes());

        let mut kids = Vec::new();
        for conteudo in &self.paginas {
            let mut c = format!("<< /Length {} /Filter /FlateDecode >>\nstream\n", conteudo.len()).into_bytes();
            c.extend_from_slice(conteudo);
            c.extend_from_slice(b"\nendstream");
            let cid = novo(&mut objs, c);
            let pid = novo(&mut objs, format!(
                "<< /Type /Page /Parent {paginas_id} 0 R /MediaBox [0 0 {:.2} {:.2}] /Resources {recursos} 0 R /Contents {cid} 0 R >>",
                A4.0 * MM, A4.1 * MM
            ).into_bytes());
            kids.push(format!("{pid} 0 R"));
        }
        objs[paginas_id - 1] = format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), kids.len()).into_bytes();
        objs[catalogo - 1] = format!("<< /Type /Catalog /Pages {paginas_id} 0 R >>").into_bytes();
        let info = novo(&mut objs, format!("<< /Title ({}) /Producer (Bingo Digital) >>", String::from_utf8_lossy(&win_ansi(titulo))).into_bytes());

        let mut out = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut posicoes = Vec::with_capacity(objs.len());
        for (i, o) in objs.iter().enumerate() {
            posicoes.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(o);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
        for p in posicoes {
            out.extend_from_slice(format!("{p:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root {catalogo} 0 R /Info {info} 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
        out
    }
}

// ------------------------------------------------------------------ código de barras Code 39

/// Padrões Code 39 (n = estreito, w = largo), alternando barra/espaço, 9 elementos por caractere.
fn padrao_code39(c: char) -> Option<&'static str> {
    Some(match c {
        '0' => "nnnwwnwnn", '1' => "wnnwnnnnw", '2' => "nnwwnnnnw", '3' => "wnwwnnnnn", '4' => "nnnwwnnnw",
        '5' => "wnnwwnnnn", '6' => "nnwwwnnnn", '7' => "nnnwnnwnw", '8' => "wnnwnnwnn", '9' => "nnwwnnwnn",
        'A' => "wnnnnwnnw", 'B' => "nnwnnwnnw", 'C' => "wnwnnwnnn", 'D' => "nnnnwwnnw", 'E' => "wnnnwwnnn",
        'F' => "nnwnwwnnn", 'G' => "nnnnnwwnw", 'H' => "wnnnnwwnn", 'I' => "nnwnnwwnn", 'J' => "nnnnwwwnn",
        'K' => "wnnnnnnww", 'L' => "nnwnnnnww", 'M' => "wnwnnnnwn", 'N' => "nnnnwnnww", 'O' => "wnnnwnnwn",
        'P' => "nnwnwnnwn", 'Q' => "nnnnnnwww", 'R' => "wnnnnnwwn", 'S' => "nnwnnnwwn", 'T' => "nnnnwnwwn",
        'U' => "wwnnnnnnw", 'V' => "nwwnnnnnw", 'W' => "wwwnnnnnn", 'X' => "nwnnwnnnw", 'Y' => "wwnnwnnnn",
        'Z' => "nwwnwnnnn", '-' => "nwnnnnwnw", '.' => "wwnnnnwnn", ' ' => "nwwnnnwnn", '*' => "nwnnwnwnn",
        _ => return None,
    })
}

/// Desenha o código de barras Code 39 de `texto` centrado em cx, topo em y, com largura total ~w.
pub fn code39(p: &mut Pagina, cx: f32, y: f32, w: f32, h: f32, texto: &str) {
    let completo: String = format!("*{}*", texto.to_uppercase());
    let padroes: Vec<&str> = completo.chars().filter_map(padrao_code39).collect();
    let (estreito, largo) = (1.0f32, 3.0f32);
    let unidades: f32 = padroes
        .iter()
        .map(|pd| pd.chars().map(|c| if c == 'w' { largo } else { estreito }).sum::<f32>() + estreito)
        .sum::<f32>()
        - estreito;
    let modulo = w / unidades;
    let mut x = cx - w / 2.0;
    p.cor(0.0, 0.0, 0.0);
    for (i, pd) in padroes.iter().enumerate() {
        for (k, c) in pd.chars().enumerate() {
            let larg = if c == 'w' { largo } else { estreito } * modulo;
            if k % 2 == 0 {
                p.retangulo(x, y, larg, h, true, false);
            }
            x += larg;
        }
        if i + 1 < padroes.len() {
            x += estreito * modulo;
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn gera_pdf_valido_e_deterministico() {
        let montar = || {
            let mut d = Documento::novo();
            let mut p = d.nova_pagina();
            p.cor(0.0, 0.0, 0.0).texto(20.0, 20.0, "1º Prêmio — Ação", Fonte::Negrito, 14.0);
            code39(&mut p, 105.0, 40.0, 50.0, 10.0, "0001");
            d.adicionar(p);
            d.gerar("Teste")
        };
        let a = montar();
        assert!(a.starts_with(b"%PDF-1.4"));
        assert!(a.ends_with(b"%%EOF\n"));
        assert_eq!(a, montar(), "mesmos dados, mesmo arquivo");
        assert!(largura("0000", Fonte::Normal, 10.0) > 20.0);
    }
}
