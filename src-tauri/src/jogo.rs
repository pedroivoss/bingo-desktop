//! Regras do sorteio no app (funcionam offline). Espelham o SorteioService do servidor: o que o
//! app aceita aqui, o servidor aceita ao sincronizar; o servidor confere tudo de novo.
//!
//! O estado da festa é o mesmo JSON que vem de GET /festas/{id}:
//! rodadas[] { premio_atual_id, pedras[{numero,ordem,automatico}], premios[{id,titulo,ordem}] }.
//! O quadro é da RODADA; o prêmio escolhido é só o que aparece no telão. Entre um prêmio e outro o
//! operador limpa o quadro. Quem conduz o bingo decide prêmios e ganhadores.

use crate::db::Cartela;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

type R<T> = Result<T, String>;

pub fn letra(numero: i64) -> &'static str {
    ["B", "I", "N", "G", "O"][((numero - 1) / 15).clamp(0, 4) as usize]
}

pub fn rodada_mut(estado: &mut Value, rodada_id: i64) -> R<&mut Value> {
    estado["rodadas"]
        .as_array_mut()
        .and_then(|rs| rs.iter_mut().find(|r| r["id"].as_i64() == Some(rodada_id)))
        .ok_or_else(|| "Rodada não encontrada.".to_string())
}

pub fn rodada<'a>(estado: &'a Value, rodada_id: i64) -> R<&'a Value> {
    estado["rodadas"]
        .as_array()
        .and_then(|rs| rs.iter().find(|r| r["id"].as_i64() == Some(rodada_id)))
        .ok_or_else(|| "Rodada não encontrada.".to_string())
}

/// Prêmio no telão (só informativo; None = nenhum).
pub fn exibir(rodada: &mut Value, premio_id: Option<i64>) -> R<()> {
    if let Some(id) = premio_id {
        let existe = rodada["premios"].as_array().map(|ps| ps.iter().any(|p| p["id"].as_i64() == Some(id))).unwrap_or(false);
        if !existe {
            return Err("Prêmio não encontrado nesta rodada.".into());
        }
    }
    rodada["premio_atual_id"] = premio_id.map(|i| json!(i)).unwrap_or(Value::Null);
    Ok(())
}

fn pedras(rodada: &Value) -> Vec<Value> {
    rodada["pedras"].as_array().cloned().unwrap_or_default()
}

pub fn inclui(rodada: &Value, numero: i64) -> bool {
    if rodada["tipo"] == "principal" {
        return true;
    }
    let ini = rodada["cartela_inicio"].as_i64().unwrap_or(1);
    let fim = rodada["cartela_fim"].as_i64().unwrap_or(i64::MAX);
    numero >= ini && numero <= fim
}

pub fn faixa(rodada: &Value) -> Option<(i64, i64)> {
    if rodada["tipo"] == "principal" {
        None
    } else {
        Some((rodada["cartela_inicio"].as_i64().unwrap_or(1), rodada["cartela_fim"].as_i64().unwrap_or(i64::MAX)))
    }
}

/// numero -> ordem (1 = primeira) das pedras do quadro da rodada.
pub fn posicoes(rodada: &Value) -> Vec<(i64, i64)> {
    let mut v: Vec<(i64, i64)> = pedras(rodada)
        .iter()
        .map(|p| (p["numero"].as_i64().unwrap_or(0), p["ordem"].as_i64().unwrap_or(0)))
        .collect();
    v.sort_by_key(|(_, o)| *o);
    v.iter().enumerate().map(|(i, (n, _))| (*n, i as i64 + 1)).collect()
}

/// (acertos, total, pedra em que completou)
pub fn situacao(cartela: &Cartela, posicoes: &[(i64, i64)]) -> (i64, i64, Option<i64>) {
    let (mut acertos, mut total, mut ultima) = (0, 0, 0);
    for coluna in &cartela.numeros {
        for n in coluna.iter().flatten() {
            total += 1;
            if let Some((_, ordem)) = posicoes.iter().find(|(x, _)| x == n) {
                acertos += 1;
                ultima = ultima.max(*ordem);
            }
        }
    }
    (acertos, total, if total > 0 && acertos == total { Some(ultima) } else { None })
}

// ------------------------------------------------------------------ ações

pub fn pedra(rodada: &mut Value, numero: i64, automatico: bool) -> R<i64> {
    if !(1..=75).contains(&numero) {
        return Err("Pedra inválida.".into());
    }
    let mut lista = pedras(rodada);
    if lista.iter().any(|p| p["numero"].as_i64() == Some(numero)) {
        return Err("Número já foi registrado.".into());
    }
    let ordem = lista.iter().filter_map(|p| p["ordem"].as_i64()).max().unwrap_or(0) + 1;
    lista.push(json!({ "numero": numero, "ordem": ordem, "automatico": automatico }));
    rodada["pedras"] = json!(lista);
    Ok(ordem)
}

/// Remove uma pedra marcada à mão (None = a última). Pedra do sorteio automático não sai.
pub fn remover(rodada: &mut Value, numero: Option<i64>) -> R<i64> {
    let lista = pedras(rodada);
    let alvo = match numero {
        Some(n) => lista.iter().find(|p| p["numero"].as_i64() == Some(n)),
        None => lista.iter().max_by_key(|p| p["ordem"].as_i64().unwrap_or(0)),
    }
    .cloned()
    .ok_or("Pedra não encontrada.")?;

    if alvo["automatico"].as_bool() == Some(true) {
        return Err("Pedra sorteada pelo sistema não pode ser removida.".into());
    }
    let numero = alvo["numero"].as_i64().unwrap_or(0);
    rodada["pedras"] = json!(lista.into_iter().filter(|p| p["numero"].as_i64() != Some(numero)).collect::<Vec<_>>());
    Ok(numero)
}

/// Limpa o quadro para o próximo prêmio (o servidor guarda as pedras no registro).
pub fn limpar(rodada: &mut Value) -> R<usize> {
    let n = pedras(rodada).len();
    if n == 0 {
        return Err("O quadro já está limpo.".into());
    }
    rodada["pedras"] = json!([]);
    Ok(n)
}

// ------------------------------------------------------------------ desenho da cartela

/// Mesma ordem do servidor (Cartela::colunasPara): rodada extra embaralhada reordena cada coluna
/// por sha256("{semente}|{codigo}|{coluna}|{numero}"); o centro livre volta ao meio da coluna N.
pub fn colunas_para(cartela: &Cartela, rodada: &Value) -> Vec<Vec<Option<i64>>> {
    let embaralhar = rodada["embaralhar"].as_bool() == Some(true) && rodada["tipo"] != "principal";
    if !embaralhar {
        return cartela.numeros.clone();
    }
    let semente = rodada["semente"].as_str().map(String::from).unwrap_or_else(|| rodada["id"].to_string());
    cartela
        .numeros
        .iter()
        .enumerate()
        .map(|(c, coluna)| {
            let mut valores: Vec<i64> = coluna.iter().flatten().copied().collect();
            valores.sort_by_cached_key(|n| hex::encode(Sha256::digest(format!("{semente}|{}|{c}|{n}", cartela.codigo).as_bytes())));
            if c == 2 && valores.len() == 4 {
                vec![Some(valores[0]), Some(valores[1]), None, Some(valores[2]), Some(valores[3])]
            } else {
                valores.into_iter().map(Some).collect()
            }
        })
        .collect()
}

pub fn linhas_para(cartela: &Cartela, rodada: &Value) -> Vec<Vec<Option<i64>>> {
    let colunas = colunas_para(cartela, rodada);
    (0..5).map(|l| (0..5).map(|c| colunas.get(c).and_then(|col| col.get(l)).copied().flatten()).collect()).collect()
}

// ------------------------------------------------------------------ demonstração

/// Semente da demonstração. NÃO MUDE: as 40 cartelas são as mesmas em todo computador e em
/// toda versão do app (um PDF de demonstração impresso hoje continua valendo amanhã).
const SEMENTE_DEMO: &str = "bingo-digital-demo-v1";

/// Número pseudoaleatório estável: os 8 primeiros bytes do SHA-256 do texto.
fn hash_estavel(texto: &str) -> u64 {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(texto.as_bytes());
    u64::from_be_bytes(d[..8].try_into().unwrap())
}

/// Cartelas da demonstração (sem conta, sem internet): sempre as mesmas para todo mundo.
/// Cada coluna pega os números da faixa (B 1–15, I 16–30...) na ordem do hash "semente|cartela|coluna|número".
pub fn gerar_cartelas_demo(quantidade: i64) -> Vec<Cartela> {
    const LETRAS: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
    (1..=quantidade)
        .map(|n| {
            let numeros = (0..5)
                .map(|c| {
                    let mut faixa: Vec<i64> = (c * 15 + 1..=c * 15 + 15).collect();
                    faixa.sort_by_key(|v| hash_estavel(&format!("{SEMENTE_DEMO}|{n}|{c}|{v}")));
                    let mut col: Vec<Option<i64>> = faixa.into_iter().take(if c == 2 { 4 } else { 5 }).map(Some).collect();
                    col.sort();
                    if c == 2 {
                        col.insert(2, None);
                    }
                    col
                })
                .collect();
            let verificador: String = (0..5)
                .map(|i| LETRAS[(hash_estavel(&format!("{SEMENTE_DEMO}|{n}|verificador|{i}")) % LETRAS.len() as u64) as usize] as char)
                .collect();
            Cartela { codigo: format!("{n:04}"), numero: n, numeros, cancelada: false, motivo: None, verificador: Some(verificador) }
        })
        .collect()
}

#[cfg(test)]
mod testes {
    use super::*;

    fn rodada_teste() -> Value {
        json!({ "id": 1, "nome": "Principal", "tipo": "principal", "premio_atual_id": null, "pedras": [], "premios": [
            { "id": 10, "titulo": "Moto", "ordem": 1 }, { "id": 11, "titulo": "TV", "ordem": 2 }
        ]})
    }

    #[test]
    fn regras_do_jogo() {
        let mut r = rodada_teste();
        let c = &gerar_cartelas_demo(1)[0];
        assert_eq!(pedra(&mut r, 9, false).unwrap(), 1, "sem prêmio escolhido, marca normalmente");
        exibir(&mut r, Some(11)).unwrap();
        exibir(&mut r, Some(10)).unwrap(); // troca à vontade, o quadro continua
        assert!(exibir(&mut r, Some(99)).is_err());
        exibir(&mut r, None).unwrap();
        assert_eq!(posicoes(&r), vec![(9, 1)]);
        assert!(pedra(&mut r, 76, false).is_err());
        assert!(pedra(&mut r, 9, false).is_err(), "repetida");
        assert_eq!(pedra(&mut r, 5, true).unwrap(), 2);
        assert!(remover(&mut r, Some(5)).unwrap_err().contains("sistema"), "automática não sai");
        assert_eq!(remover(&mut r, Some(9)).unwrap(), 9);
        assert_eq!(limpar(&mut r).unwrap(), 1, "o quadro limpa mesmo com pedra automática");
        assert!(limpar(&mut r).is_err());
        for n in c.numeros.iter().flatten().flatten() {
            pedra(&mut r, *n, false).unwrap();
        }
        assert_eq!(situacao(c, &posicoes(&r)).2, Some(24), "bateu na 24ª");
        assert_eq!(c.verificador.as_ref().map(|v| v.len()), Some(5));
    }

    const FIXO_CARTELA_1: &str = "[[Some(1), Some(8), Some(10), Some(11), Some(14)], [Some(16), Some(19), Some(22), Some(29), Some(30)], [Some(31), Some(34), None, Some(42), Some(44)], [Some(49), Some(50), Some(53), Some(55), Some(58)], [Some(61), Some(66), Some(67), Some(68), Some(75)]]";
    const FIXO_VERIFICADOR_1: &str = "9KYKJ";

    #[test]
    fn demonstracao_igual_para_todos() {
        let a = gerar_cartelas_demo(40);
        assert_eq!(a.len(), 40);
        let resumo = |v: &[Cartela]| v.iter().map(|c| (c.numeros.clone(), c.verificador.clone())).collect::<Vec<_>>();
        assert_eq!(resumo(&a), resumo(&gerar_cartelas_demo(40)), "sempre as mesmas");
        let unicas: std::collections::HashSet<String> = a.iter().map(|c| format!("{:?}", c.numeros)).collect();
        assert_eq!(unicas.len(), 40, "nenhuma cartela repetida");
        for c in &a {
            for (i, col) in c.numeros.iter().enumerate() {
                let i = i as i64;
                let nums: Vec<i64> = col.iter().flatten().copied().collect();
                assert_eq!(nums.len(), if i == 2 { 4 } else { 5 });
                assert!(nums.iter().all(|v| (i * 15 + 1..=i * 15 + 15).contains(v)), "faixa da coluna");
                assert!(nums.windows(2).all(|w| w[0] < w[1]), "crescente e sem repetir");
            }
            assert!(c.numeros[2][2].is_none(), "centro livre");
        }
        // trava a semente: se este teste quebrar, as cartelas impressas da demonstração mudaram
        assert_eq!(a[0].numeros, gerar_cartelas_demo(1)[0].numeros);
        assert_eq!(format!("{:?}", a[0].numeros), FIXO_CARTELA_1);
        assert_eq!(a[0].verificador.as_deref(), Some(FIXO_VERIFICADOR_1));
    }

    #[test]
    fn embaralha_igual_ao_servidor() {
        // mesmo cálculo do PHP: hash('sha256', "{semente}|{codigo}|{c}|{n}") em ordem crescente
        let c = Cartela { codigo: "0001".into(), numero: 1, cancelada: false, motivo: None, verificador: None,
            numeros: vec![vec![Some(1), Some(2), Some(3), Some(4), Some(5)], vec![Some(16), Some(17), Some(18), Some(19), Some(20)],
                vec![Some(31), Some(32), None, Some(33), Some(34)], vec![Some(46), Some(47), Some(48), Some(49), Some(50)],
                vec![Some(61), Some(62), Some(63), Some(64), Some(65)]] };
        let r = json!({ "id": 2, "tipo": "extra", "embaralhar": true, "semente": "abc" });
        let cols = colunas_para(&c, &r);
        // resultado do PHP (Cartela::colunasPara) para a mesma cartela, semente "abc"
        let php: Vec<Vec<Option<i64>>> = serde_json::from_str("[[3,2,5,4,1],[20,16,18,19,17],[32,31,null,34,33],[47,49,48,46,50],[64,63,62,65,61]]").unwrap();
        assert_eq!(cols, php, "o app embaralha exatamente como o servidor");
    }
}
