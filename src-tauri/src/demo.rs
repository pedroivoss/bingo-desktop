//! Demonstração (sem conta, sem internet): a mesma festa de exemplo da página inicial do site.
//!
//! - Principal: 20 cartelas em folha inteira, 6 prêmios (uma grade por prêmio).
//! - Extra 1: cartelas 1–4 embaralhadas, folha inteira, "EXTRA 1" na lateral.
//! - Extra 2: cartelas 5–8 embaralhadas, meia folha (2 folhas com 2 cartelas e linha de corte).
//!
//! As artes (cabeçalho, Pix, selo, patrocinadores e "SUA LOGO") vêm dentro do app: foram desenhadas
//! uma vez com o mesmo visual da prévia da home (resources/views/components/cartela-exemplo.blade.php).

use crate::cartelas_pdf::Arte;
use serde_json::{json, Value};

/// Versão da demonstração: 1 = 40 cartelas sem artes; 2 = 20 cartelas + extras no modelo da home.
/// Quem abrir uma demonstração de versão anterior recebe esta. As cartelas não mudam (mesma semente).
pub const VERSAO: i64 = 2;
pub const CARTELAS: i64 = 20;

const CABECALHO: &[u8] = include_bytes!("../demo/cabecalho.png");
const SUA_LOGO: &[u8] = include_bytes!("../demo/sua_logo.png");
const QR_PIX: &[u8] = include_bytes!("../demo/qr_pix.png");
const QR_SITE: &[u8] = include_bytes!("../demo/qr_site.png");
const PATROCINADORES: [(&str, &[u8]); 8] = [
    ("Mercado Bom Preço", include_bytes!("../demo/patrocinador_01.png")),
    ("Padaria Pão Quente", include_bytes!("../demo/patrocinador_02.png")),
    ("Auto Peças Central", include_bytes!("../demo/patrocinador_03.png")),
    ("Farmácia Vida", include_bytes!("../demo/patrocinador_04.png")),
    ("Loja Estrela", include_bytes!("../demo/patrocinador_05.png")),
    ("Posto Avenida", include_bytes!("../demo/patrocinador_06.png")),
    ("Açougue Boi Bom", include_bytes!("../demo/patrocinador_07.png")),
    ("Ótica Visão", include_bytes!("../demo/patrocinador_08.png")),
];

pub fn arte() -> Arte {
    Arte {
        cabecalho: Some(CABECALHO.to_vec()),
        marca_dagua: None,
        coringa: None,
        logo: Some(SUA_LOGO.to_vec()),
        pix: Some(QR_PIX.to_vec()),
        selo_qr: Some(QR_SITE.to_vec()),
        patrocinadores: PATROCINADORES.iter().map(|(_, b)| b.to_vec()).collect(),
    }
}

/// Patrocinadores para o carrossel do telão: (nome, PNG).
pub fn patrocinadores() -> impl Iterator<Item = (&'static str, &'static [u8])> {
    PATROCINADORES.iter().copied()
}

/// Estado inicial da festa de demonstração (sem pedras nem vencedores).
pub fn estado() -> Value {
    let premio = |id: i64, titulo: &str, ordem: i64| json!({ "id": id, "titulo": titulo, "ordem": ordem, "encerrado_em": null, "pedras_removidas": 0, "reinicios": 0, "pedras": [], "vencedores": [] });
    let principais = ["Moto 0 km", "Geladeira", "Air fryer", "TV 50\"", "Bicicleta", "R$ 500 no Pix"];
    json!({
        "festa": {
            "id": 0, "nome": "Demonstração", "sorteio_automatico": true, "regra_empate_descricao": "Combine com o público antes de começar",
            "total_cartelas": CARTELAS, "demo": true, "demo_versao": VERSAO,
            "rodape": "Paróquia São José · Rua das Flores, 100",
            "pix_texto": "Pix: R$ 10,00 (dez reais)",
            "layout": { "cabecalho": "imagem", "patrocinadores": "logos", "marca_grande": false, "chancela": false },
            "selo": { "modo": "normal", "texto": "Cartelas geradas pelo Bingo Digital", "site": "bingodigital.ivocompany.com" },
        },
        "rodadas": [
            { "id": 1, "nome": "Principal", "tipo": "principal", "ordem": 0, "embaralhar": false, "formato": "folha",
              "premios": principais.iter().enumerate().map(|(i, t)| premio(i as i64 + 1, t, i as i64 + 1)).collect::<Vec<_>>() },
            { "id": 2, "nome": "Extra 1", "tipo": "extra", "ordem": 1, "cartela_inicio": 1, "cartela_fim": 4, "embaralhar": true,
              "semente": "bingo-digital-demo-extra-1", "marca_extra": true, "formato": "folha",
              "premios": [premio(7, "Cesta de Natal", 1)] },
            { "id": 3, "nome": "Extra 2", "tipo": "extra", "ordem": 2, "cartela_inicio": 5, "cartela_fim": 8, "embaralhar": true,
              "semente": "bingo-digital-demo-extra-2", "marca_extra": true, "formato": "meia_folha",
              "premios": [premio(8, "Bicicleta aro 29", 1)] },
        ]
    })
}

#[cfg(test)]
mod testes {
    use super::*;
    use crate::{cartelas_pdf, jogo};

    #[test]
    fn artes_embutidas_sao_imagens_validas() {
        let a = arte();
        for b in [&a.cabecalho, &a.logo, &a.pix, &a.selo_qr] {
            assert!(crate::pdf::Imagem::carregar(b.as_ref().unwrap()).is_some());
        }
        assert_eq!(a.patrocinadores.len(), 8);
        assert!(a.patrocinadores.iter().all(|b| crate::pdf::Imagem::carregar(b).is_some()));
    }

    #[test]
    fn modelos_da_demonstracao() {
        let est = estado();
        let cartelas = jogo::gerar_cartelas_demo(CARTELAS);
        let gerar = |id: i64| {
            let rodada = jogo::rodada(&est, id).unwrap();
            let premios = rodada["premios"].as_array().unwrap().iter().map(|p| (p["ordem"].as_i64().unwrap(), p["titulo"].as_str().unwrap().to_string())).collect();
            let (ini, fim) = jogo::faixa(rodada).unwrap_or((1, CARTELAS));
            let lote = &cartelas[(ini - 1) as usize..fim.min(CARTELAS) as usize]; // cartelas 1..=20, em ordem
            let layout = cartelas_pdf::Layout { mostrar_rodape: true, pix_texto: Some("Pix: R$ 10,00 (dez reais)".into()),
                selo: Some(cartelas_pdf::Selo { grande: false, texto: "Cartelas geradas pelo Bingo Digital".into(), site: "bingodigital.ivocompany.com".into() }), ..Default::default() };
            let ctx = cartelas_pdf::Contexto { festa_nome: "Demonstração", rodape: "Paróquia São José".into(), rodada, premios, rastreio: "DEMONSTRAÇÃO".into(), layout };
            cartelas_pdf::gerar(&ctx, &arte(), lote)
        };
        let (_, paginas, _) = gerar(1);
        assert_eq!(paginas, 20, "principal: uma folha por cartela");
        let (_, paginas, _) = gerar(2);
        assert_eq!(paginas, 4, "Extra 1: 4 cartelas em folha inteira");
        let (_, paginas, det) = gerar(3);
        assert_eq!(paginas, 2, "Extra 2: 4 cartelas em 2 folhas");
        assert_eq!(det.unwrap(), "cima 0005–0006 · baixo 0007–0008");
        if let Ok(pasta) = std::env::var("AMOSTRA_PDF") {
            for (id, nome) in [(1, "principal"), (2, "extra1"), (3, "extra2")] {
                std::fs::write(format!("{pasta}/demo_{nome}.pdf"), gerar(id).0).unwrap();
            }
        }
    }
}
