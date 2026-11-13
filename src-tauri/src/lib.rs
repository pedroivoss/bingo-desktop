//! Bingo Digital — app desktop.
//!
//! - Login da conta (vira um "computador" com token próprio) e licença ASSINADA pelo servidor,
//!   conferida aqui com a chave pública embutida (ver licenca.rs).
//! - Funciona sem internet até `valida_ate` (10 dias desde o último contato). Passou disso, ou
//!   relógio do computador voltado para trás, ou conta bloqueada: apaga os dados sincronizados
//!   (os PDFs salvos pelo cliente não são tocados).
//! - O sorteio roda local com as mesmas regras do servidor (jogo.rs) e vira uma fila de eventos,
//!   enviada ao sincronizar. O servidor confere tudo de novo e é quem manda no final.
//! - Demonstração: 40 cartelas geradas no próprio computador, sem conta e sem internet.

mod atualizacao;
mod cartelas_pdf;
mod db;
mod jogo;
mod licenca;
mod pdf;

use chrono::{DateTime, Duration, Utc};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::sync::Mutex;
use sha2::Digest;
use tauri::{Emitter, Manager, State};

const FESTA_DEMO: i64 = 0;
/// Versão do app: a mesma que o atualizador usa (a do pacote, tauri.conf.json), gravada ao iniciar.
/// Não usar a do Cargo.toml: se as duas divergirem, o app se apresentaria ao servidor com uma versão
/// e um executável de outra, e o painel marcaria "app alterado" em todos os computadores.
static VERSAO_APP: std::sync::OnceLock<String> = std::sync::OnceLock::new();

fn versao() -> &'static str {
    VERSAO_APP.get().map(String::as_str).unwrap_or(env!("CARGO_PKG_VERSION"))
}

struct Estado {
    db: Mutex<Connection>,
    http: reqwest::Client,
    pasta: std::path::PathBuf, // pasta de dados do app (banco e artes da festa em cache)
    app_hash: String,          // impressão digital do executável (app adulterado = hash diferente)
    /// Documentos/Bingo Digital: pasta do cliente com as imagens e os PDFs de cada festa.
    documentos: Option<std::path::PathBuf>,
}

/// SHA-256 do próprio executável. Vai em toda chamada à API: o servidor compara com as versões
/// oficiais e marca no painel do admin computadores rodando um app modificado.
fn hash_do_executavel() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .map(|b| hex::encode(sha2::Sha256::digest(&b)))
        .unwrap_or_default()
}

/// Cliente HTTP. No build de produção com BINGO_FIXAR_CERTIFICADO=letsencrypt, o app SÓ confia nas
/// raízes do Let's Encrypt (embutidas): programas que interceptam o tráfego instalando um
/// certificado próprio no computador (para estudar ou falsificar a API) deixam de funcionar.
fn cliente_http() -> reqwest::Result<reqwest::Client> {
    cliente_com(option_env!("BINGO_FIXAR_CERTIFICADO") == Some("letsencrypt"), base_url().starts_with("https://"))
}

fn cliente_com(fixar: bool, so_https: bool) -> reqwest::Result<reqwest::Client> {
    let mut b = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).https_only(so_https);
    if fixar {
        b = b.tls_built_in_root_certs(false);
        for pem in [include_bytes!("../certs/isrg-root-x1.pem").as_slice(), include_bytes!("../certs/isrg-root-x2.pem").as_slice()] {
            b = b.add_root_certificate(reqwest::Certificate::from_pem(pem)?);
        }
    }
    b.build()
}

pub(crate) fn base_url() -> String {
    let url = option_env!("BINGO_API_URL").unwrap_or("http://ivocompany.local:8000/src/Bingo/public");
    format!("{}/api/desktop/v1", url.trim_end_matches('/'))
}

type R<T> = Result<T, String>;

fn erro(codigo: &str, mensagem: &str) -> String {
    json!({ "codigo": codigo, "message": mensagem }).to_string()
}

// ------------------------------------------------------------------ HTTP

async fn chamar(st: &Estado, metodo: reqwest::Method, caminho: &str, token: Option<&str>, corpo: Option<Value>) -> R<(u16, Value)> {
    let mut req = st
        .http
        .request(metodo, format!("{}{}", base_url(), caminho))
        .header("Accept", "application/json")
        .header("X-App-Versao", versao())
        .header("X-App-Hash", &st.app_hash)
        .header("X-App-Plataforma", std::env::consts::OS);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    if let Some(c) = corpo {
        req = req.json(&c);
    }
    let resp = req.send().await.map_err(|_| erro("sem_internet", "Sem conexão com o servidor. O app continua funcionando offline."))?;
    let status = resp.status().as_u16();
    let dados: Value = resp.json().await.unwrap_or(json!({}));
    Ok((status, dados))
}

/// Resposta de erro da API → texto para a tela. Token morto: apaga a conta local.
fn tratar_erro(st: &Estado, status: u16, dados: &Value) -> String {
    let codigo = dados["codigo"].as_str().unwrap_or("erro");
    if matches!(codigo, "token_invalido" | "dispositivo_revogado" | "conta_desativada") {
        db::apagar_dados_da_conta(&st.db.lock().unwrap());
    }
    let mensagem = dados["message"].as_str().map(String::from).unwrap_or_else(|| format!("Erro {status} no servidor."));
    json!({ "codigo": codigo, "message": mensagem, "dispositivos": dados["dispositivos"] }).to_string()
}

fn token(conn: &Connection) -> R<String> {
    db::kv_get(conn, "token").ok_or_else(|| erro("sem_login", "Entre com a sua conta."))
}

// ------------------------------------------------------------------ licença

fn agora() -> DateTime<Utc> {
    Utc::now()
}

/// Atualiza o "maior horário já visto" (relógio do computador não pode voltar).
fn registrar_horario(conn: &Connection, servidor: Option<&str>) {
    let mut maior = db::kv_get(conn, "maior_horario").and_then(|s| DateTime::parse_from_rfc3339(&s).ok()).map(|d| d.with_timezone(&Utc)).unwrap_or(agora());
    maior = maior.max(agora());
    if let Some(s) = servidor.and_then(|s| DateTime::parse_from_rfc3339(s).ok()) {
        maior = maior.max(s.with_timezone(&Utc));
    }
    let _ = db::kv_set(conn, "maior_horario", &maior.to_rfc3339());
}

fn salvar_licenca(conn: &Connection, licenca: &str) -> R<Value> {
    let dados = licenca::conferir(licenca)?;
    db::kv_set(conn, "licenca", licenca)?;
    registrar_horario(conn, dados["servidor_agora"].as_str());
    Ok(dados)
}

/// Licença válida agora (assinatura, prazo offline, relógio e bloqueio). Se não vale mais,
/// apaga os dados sincronizados e explica o motivo.
fn licenca_valida(conn: &Connection) -> R<Value> {
    let Some(texto) = db::kv_get(conn, "licenca") else {
        return Err(erro("sem_login", "Entre com a sua conta."));
    };
    let invalida = |motivo: &str, msg: &str| {
        db::apagar_dados_da_conta(conn);
        Err(erro(motivo, msg))
    };

    let dados = match licenca::conferir(&texto) {
        Ok(d) => d,
        Err(_) => return invalida("licenca_invalida", "A licença salva neste computador não é válida. Entre de novo."),
    };
    if dados["bloqueada"].as_bool() == Some(true) {
        return invalida("bloqueada", "Sua licença venceu ou a conta está bloqueada. Renove no site e entre de novo.");
    }

    let maior = db::kv_get(conn, "maior_horario").and_then(|s| DateTime::parse_from_rfc3339(&s).ok()).map(|d| d.with_timezone(&Utc));
    if let Some(maior) = maior {
        if agora() < maior - Duration::minutes(10) {
            return invalida("relogio", "O relógio deste computador está atrasado em relação ao último uso. Acerte a data e a hora e entre de novo.");
        }
    }

    let valida_ate = dados["valida_ate"].as_str().and_then(|s| DateTime::parse_from_rfc3339(s).ok()).map(|d| d.with_timezone(&Utc));
    match valida_ate {
        Some(ate) if agora() <= ate => {
            registrar_horario(conn, None);
            Ok(dados)
        }
        _ => invalida("expirada", "Faz mais de 10 dias que este computador não fala com a internet. Os dados foram apagados por segurança; conecte e entre de novo (os PDFs salvos continuam no computador)."),
    }
}

fn exigir_festa_liberada(conn: &Connection, festa_id: i64) -> R<()> {
    if festa_id == FESTA_DEMO {
        return Ok(());
    }
    let lic = licenca_valida(conn)?;
    let festa = lic["festas"].as_array().and_then(|f| f.iter().find(|f| f["id"].as_i64() == Some(festa_id)).cloned());
    match festa {
        Some(f) if f["pode_sortear"].as_bool() == Some(true) => Ok(()),
        Some(_) => Err(erro("licenca", "Esta festa tem mais cartelas do que o plano da licença permite: sorteio bloqueado.")),
        None => Err(erro("festa", "Esta festa não está mais liberada para esta conta.")),
    }
}

// ------------------------------------------------------------------ comandos: conta

pub(crate) fn situacao_impl(st: &Estado) -> Value {
    let conn = st.db.lock().unwrap();
    let pendentes = db::eventos_pendentes(&conn, None).len();
    let demo = db::festa_get(&conn, FESTA_DEMO).is_some();
    match licenca_valida(&conn) {
        Ok(lic) => json!({ "logado": true, "licenca": lic, "pendentes": pendentes, "demo": demo, "versao": versao(), "servidor": base_url() }),
        Err(e) => json!({ "logado": false, "motivo": serde_json::from_str::<Value>(&e).unwrap_or(json!({})), "demo": demo, "versao": versao(), "servidor": base_url() }),
    }
}

pub(crate) async fn login_impl(st: &Estado, email: String, senha: String, codigo: Option<String>) -> R<Value> {
    let (uuid, nome, plataforma) = {
        let conn = st.db.lock().unwrap();
        let uuid = db::kv_get(&conn, "uuid").unwrap_or_else(|| {
            let novo = uuid::Uuid::new_v4().to_string();
            let _ = db::kv_set(&conn, "uuid", &novo);
            novo
        });
        let nome = gethostname::gethostname().to_string_lossy().chars().take(120).collect::<String>();
        let plataforma = match std::env::consts::OS { "macos" => "macos", "windows" => "windows", _ => "linux" };
        (uuid, nome, plataforma)
    };

    let corpo = json!({ "email": email, "password": senha, "codigo": codigo, "uuid": uuid, "nome": nome, "plataforma": plataforma, "app_versao": versao() });
    let (status, dados) = chamar(&st, reqwest::Method::POST, "/login", None, Some(corpo)).await?;
    if status != 200 {
        return Err(tratar_erro(&st, status, &dados));
    }

    let conn = st.db.lock().unwrap();
    db::kv_set(&conn, "token", dados["token"].as_str().unwrap_or_default())?;
    salvar_licenca(&conn, dados["licenca"].as_str().unwrap_or_default()).map_err(|e| erro("licenca_invalida", &e))?;
    Ok(json!({ "ok": true }))
}

pub(crate) async fn sair_impl(st: &Estado) -> R<Value> {
    let tok = { db::kv_get(&st.db.lock().unwrap(), "token") };
    if let Some(t) = tok {
        let _ = chamar(&st, reqwest::Method::POST, "/logout", Some(&t), None).await;
    }
    db::apagar_dados_da_conta(&st.db.lock().unwrap());
    Ok(json!({ "ok": true }))
}

/// Licença nova (o app chama ao abrir e de tempos em tempos).
pub(crate) async fn renovar_impl(st: &Estado) -> R<Value> {
    let tok = { token(&st.db.lock().unwrap())? };
    let (status, dados) = chamar(&st, reqwest::Method::GET, "/licenca", Some(&tok), None).await?;
    if status != 200 {
        return Err(tratar_erro(&st, status, &dados));
    }
    let conn = st.db.lock().unwrap();
    salvar_licenca(&conn, dados["licenca"].as_str().unwrap_or_default()).map_err(|e| erro("licenca_invalida", &e))
}

// ------------------------------------------------------------------ comandos: festa

/// Envia os eventos pendentes e baixa a festa e as cartelas (incremental). Avisa o progresso.
pub(crate) async fn sincronizar_impl(st: &Estado, festa_id: i64, progresso: &(dyn Fn(Value) + Send + Sync)) -> R<Value> {
    if festa_id == FESTA_DEMO {
        return Ok(json!({ "enviados": 0 }));
    }
    let tok = {
        let conn = st.db.lock().unwrap();
        licenca_valida(&conn)?;
        token(&conn)?
    };

    // 1) envia o que foi feito offline, em ordem
    let pendentes = { db::eventos_pendentes(&st.db.lock().unwrap(), Some(festa_id)) };
    progresso(json!({ "etapa": "enviando", "feitas": 0, "total": pendentes.len() }));
    let mut enviados = 0;
    let mut recusado: Option<String> = None;
    for lote in pendentes.chunks(500) {
        let eventos: Vec<Value> = lote.iter().filter_map(|(_, corpo)| serde_json::from_str(corpo).ok()).collect();
        let (status, dados) = chamar(&st, reqwest::Method::POST, &format!("/festas/{festa_id}/eventos"), Some(&tok), Some(json!({ "eventos": eventos }))).await?;
        if status != 200 {
            return Err(tratar_erro(&st, status, &dados));
        }
        let conn = st.db.lock().unwrap();
        for r in dados["resultados"].as_array().cloned().unwrap_or_default() {
            let uuid = r["uuid"].as_str().unwrap_or_default();
            match r["status"].as_str() {
                Some("ok") => {
                    db::evento_status(&conn, uuid, "enviado", None);
                    enviados += 1;
                }
                Some("recusado") => {
                    let msg = r["message"].as_str().unwrap_or("Recusado pelo servidor.");
                    db::evento_status(&conn, uuid, "recusado", Some(msg));
                    recusado.get_or_insert_with(|| msg.to_string());
                }
                _ => db::evento_status(&conn, uuid, "descartado", Some("não processado: veio depois de um evento recusado")),
            }
        }
        if recusado.is_some() {
            break;
        }
    }
    if recusado.is_some() {
        // o que ficou na fila depende do evento recusado: descarta (o servidor é quem manda)
        let conn = st.db.lock().unwrap();
        for (uuid, _) in db::eventos_pendentes(&conn, Some(festa_id)) {
            db::evento_status(&conn, &uuid, "descartado", Some("não enviado: veio depois de um evento recusado"));
        }
    }

    // 2) baixa a festa (o servidor é a verdade) e as cartelas
    let (status, festa) = chamar(&st, reqwest::Method::GET, &format!("/festas/{festa_id}"), Some(&tok), None).await?;
    if status != 200 {
        return Err(tratar_erro(&st, status, &festa));
    }
    { db::festa_set(&st.db.lock().unwrap(), festa_id, &festa.to_string())?; }
    baixar_artes(st, &festa, progresso).await;
    enviar_pdfs_pendentes(st, festa_id, &tok).await;

    let total = festa["festa"]["total_cartelas"].as_i64().unwrap_or(0);
    let desde = { db::kv_get(&st.db.lock().unwrap(), &format!("cartelas_desde_{festa_id}")) };
    // completo: 1ª vez, faltam cartelas ou festa baixada antes do código verificador existir
    let completo = desde.is_none() || {
        let conn = st.db.lock().unwrap();
        db::total_cartelas(&conn, festa_id) < total || db::faltam_verificadores(&conn, festa_id)
    };
    let mut depois = 0i64;
    let mut feitas = 0i64;
    let mut agora_servidor = String::new();
    loop {
        let mut url = format!("/festas/{festa_id}/cartelas?depois={depois}");
        if !completo {
            url.push_str(&format!("&alteradas_desde={}", urlencode(desde.as_deref().unwrap_or_default())));
        }
        let (status, pagina) = chamar(&st, reqwest::Method::GET, &url, Some(&tok), None).await?;
        if status != 200 {
            return Err(tratar_erro(&st, status, &pagina));
        }
        {
            let conn = st.db.lock().unwrap();
            let _ = conn.execute_batch("BEGIN");
            for c in pagina["cartelas"].as_array().cloned().unwrap_or_default() {
                let codigo = c["codigo"].as_str().unwrap_or_default().to_string();
                let cartela = db::Cartela {
                    numero: codigo.parse().unwrap_or(0),
                    codigo,
                    numeros: serde_json::from_value(c["numeros"].clone()).unwrap_or_default(),
                    cancelada: c["cancelada"].as_bool().unwrap_or(false),
                    motivo: c["motivo_cancelamento"].as_str().map(String::from),
                    verificador: c["verificador"].as_str().map(String::from),
                };
                db::cartela_set(&conn, festa_id, &cartela)?;
                feitas += 1;
            }
            let _ = conn.execute_batch("COMMIT");
        }
        if agora_servidor.is_empty() {
            agora_servidor = pagina["servidor_agora"].as_str().unwrap_or_default().to_string();
        }
        progresso(json!({ "etapa": "cartelas", "festa_id": festa_id, "feitas": feitas, "total": if completo { total } else { feitas } }));
        match pagina["proximo"].as_i64() {
            Some(p) => depois = p,
            None => break,
        }
    }
    {
        let conn = st.db.lock().unwrap();
        db::kv_set(&conn, &format!("cartelas_desde_{festa_id}"), &agora_servidor)?;
        registrar_horario(&conn, Some(&agora_servidor));
    }

    Ok(json!({ "enviados": enviados, "recusado": recusado, "cartelas": feitas }))
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn ler_estado(conn: &Connection, festa_id: i64) -> R<Value> {
    db::festa_get(conn, festa_id)
        .and_then(|s| serde_json::from_str(&s).ok())
        .ok_or_else(|| erro("nao_sincronizada", "Baixe a festa antes (precisa de internet uma vez)."))
}

pub(crate) fn estado_festa_impl(st: &Estado, festa_id: i64) -> R<Value> {
    let conn = st.db.lock().unwrap();
    if festa_id != FESTA_DEMO {
        licenca_valida(&conn)?;
    }
    let mut estado = ler_estado(&conn, festa_id)?;
    estado["total_local"] = json!(db::total_cartelas(&conn, festa_id));
    estado["pendentes"] = json!(db::eventos_pendentes(&conn, Some(festa_id)).len());
    Ok(estado)
}

/// Aplica uma ação do sorteio localmente (mesmas regras do servidor) e põe na fila de envio.
pub(crate) fn acao_impl(st: &Estado, festa_id: i64, rodada_id: i64, tipo: String, dados: Value) -> R<Value> {
    let conn = st.db.lock().unwrap();
    exigir_festa_liberada(&conn, festa_id)?;
    let mut estado = ler_estado(&conn, festa_id)?;
    let rodada = jogo::rodada_mut(&mut estado, rodada_id)?;
    let evento;
    match tipo.as_str() {
        // prêmio no telão (só informativo); premio_id nulo = nenhum
        "escolher" => {
            jogo::exibir(rodada, dados["premio_id"].as_i64())?;
            evento = json!({ "premio_id": dados["premio_id"] });
        }
        "pedra" => {
            jogo::pedra(rodada, dados["numero"].as_i64().unwrap_or(0), false)?;
            evento = json!({ "numero": dados["numero"], "automatico": false });
        }
        "remover" => {
            let n = jogo::remover(rodada, dados["numero"].as_i64())?;
            evento = json!({ "numero": n });
        }
        "limpar" => {
            jogo::limpar(rodada)?;
            evento = json!({});
        }
        _ => return Err("Ação desconhecida.".into()),
    }

    db::festa_set(&conn, festa_id, &estado.to_string())?;
    if festa_id != FESTA_DEMO {
        let uuid = uuid::Uuid::new_v4().to_string();
        let corpo = json!({ "uuid": uuid, "tipo": tipo, "rodada_id": rodada_id, "dados": evento, "ocorrido_em": agora().to_rfc3339() });
        db::evento_add(&conn, festa_id, &uuid, &corpo.to_string())?;
    }
    Ok(json!({ "ok": true }))
}

/// Sorteio automático local: gerador aleatório do sistema operacional; a pedra é escolhida ANTES
/// de olhar as cartelas (as cartelas só decidem se a revelação terá suspense).
pub(crate) fn sortear_impl(st: &Estado, festa_id: i64, rodada_id: i64) -> R<Value> {
    use rand::Rng;
    let conn = st.db.lock().unwrap();
    exigir_festa_liberada(&conn, festa_id)?;
    let mut estado = ler_estado(&conn, festa_id)?;
    if festa_id != FESTA_DEMO && estado["festa"]["sorteio_automatico"].as_bool() != Some(true) {
        return Err("O sorteio automático não está habilitado nesta festa (Controle → Configurações, no site).".into());
    }

    let rodada = jogo::rodada_mut(&mut estado, rodada_id)?;
    let saiu: Vec<i64> = jogo::posicoes(rodada).iter().map(|(n, _)| *n).collect();
    let restantes: Vec<i64> = (1..=75).filter(|n| !saiu.contains(n)).collect();
    if restantes.is_empty() {
        return Err("Todas as 75 pedras já foram sorteadas. Limpe o quadro para o próximo prêmio.".into());
    }
    let numero = restantes[rand::rngs::OsRng.gen_range(0..restantes.len())];

    // suspense: alguma cartela estava a 1 pedra? esta pedra fecha quantas?
    let (mut suspense, mut bateu) = (false, 0);
    for c in db::cartelas_ativas(&conn, festa_id, jogo::faixa(rodada)) {
        let faltando: Vec<i64> = c.numeros.iter().flatten().flatten().copied().filter(|n| !saiu.contains(n)).collect();
        if faltando.len() == 1 {
            suspense = true;
            if faltando[0] == numero {
                bateu += 1;
            }
        }
    }

    jogo::pedra(rodada, numero, true)?;
    db::festa_set(&conn, festa_id, &estado.to_string())?;
    if festa_id != FESTA_DEMO {
        let uuid = uuid::Uuid::new_v4().to_string();
        let corpo = json!({ "uuid": uuid, "tipo": "pedra", "rodada_id": rodada_id, "dados": { "numero": numero, "automatico": true }, "ocorrido_em": agora().to_rfc3339() });
        db::evento_add(&conn, festa_id, &uuid, &corpo.to_string())?;
    }
    Ok(json!({ "numero": numero, "letra": jogo::letra(numero), "suspense": suspense, "bateu": bateu }))
}

pub(crate) fn conferir_impl(st: &Estado, festa_id: i64, rodada_id: i64, codigo: String) -> R<Value> {
    let conn = st.db.lock().unwrap();
    let estado = ler_estado(&conn, festa_id)?;
    let rodada = jogo::rodada(&estado, rodada_id)?;
    // pelo número ("42", "0042") ou pelo código verificador ("7K3QX", 5 letras/números com ao menos uma letra)
    let limpo: String = codigo.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_uppercase();
    let cartela = if limpo.len() == 5 && limpo.chars().any(|c| c.is_ascii_alphabetic()) {
        db::cartela_por_verificador(&conn, festa_id, &limpo)
    } else {
        limpo.parse::<i64>().ok().and_then(|n| db::cartela_get(&conn, festa_id, n))
    }
    .ok_or("Cartela não encontrada nesta festa. Confira o número e o código impressos nela.")?;
    let premio = rodada["premios"].as_array().and_then(|ps| ps.iter().find(|p| p["id"] == rodada["premio_atual_id"])).map(|p| p["titulo"].clone());
    if !jogo::inclui(rodada, cartela.numero) {
        return Err(format!("A cartela {} não participa da rodada \"{}\".", cartela.codigo, rodada["nome"].as_str().unwrap_or("")));
    }
    let pos = jogo::posicoes(rodada);
    let (acertos, total, bateu) = jogo::situacao(&cartela, &pos);

    Ok(json!({
        "codigo": cartela.codigo,
        "verificador": cartela.verificador,
        "linhas": jogo::linhas_para(&cartela, rodada),
        "sorteados": pos.iter().map(|(n, _)| n).collect::<Vec<_>>(),
        "premio": premio,
        "total_pedras": pos.len(),
        "acertos": acertos,
        "faltam": total - acertos,
        "bateu_na_pedra": bateu,
        "cancelada": cartela.cancelada,
        "motivo_cancelamento": cartela.motivo,
    }))
}

/// Top 5 mais perto de bater com as pedras do quadro (sem revelar o código).
pub(crate) fn ranking_impl(st: &Estado, festa_id: i64, rodada_id: i64) -> R<Value> {
    let conn = st.db.lock().unwrap();
    let estado = ler_estado(&conn, festa_id)?;
    let rodada = jogo::rodada(&estado, rodada_id)?;
    let pos = jogo::posicoes(rodada);
    if pos.is_empty() {
        return Ok(json!([]));
    }
    let total = pos.len() as i64;
    let mut lista: Vec<(i64, i64)> = db::cartelas_ativas(&conn, festa_id, jogo::faixa(rodada))
        .iter()
        .filter_map(|c| {
            let (a, t, bateu) = jogo::situacao(c, &pos);
            match bateu {
                Some(p) if total - p >= 2 => None, // bateu há 2+ pedras e ninguém gritou
                _ => Some((a, t)),
            }
        })
        .collect();
    lista.sort_by(|x, y| (y.0 * x.1).cmp(&(x.0 * y.1)));
    let romanos = ["I", "II", "III", "IV", "V"];
    Ok(json!(lista.iter().take(5).enumerate().map(|(i, (a, t))| json!({
        "posicao": romanos[i], "percentual": (*a as f64 / *t as f64 * 1000.0).round() / 10.0, "faltam": t - a
    })).collect::<Vec<_>>()))
}

// ------------------------------------------------------------------ demonstração

pub(crate) fn iniciar_demo_impl(st: &Estado, recomecar: bool) -> R<Value> {
    let conn = st.db.lock().unwrap();
    if recomecar || db::festa_get(&conn, FESTA_DEMO).is_none() {
        let _ = conn.execute("DELETE FROM cartelas WHERE festa_id = 0", []);
        for c in jogo::gerar_cartelas_demo(40) {
            db::cartela_set(&conn, FESTA_DEMO, &c)?;
        }
        let premio = |id: i64, titulo: &str, ordem: i64| json!({ "id": id, "titulo": titulo, "ordem": ordem, "encerrado_em": null, "pedras_removidas": 0, "reinicios": 0, "pedras": [], "vencedores": [] });
        let estado = json!({
            "festa": { "id": 0, "nome": "Demonstração", "sorteio_automatico": true, "regra_empate_descricao": "Combine com o público antes de começar", "total_cartelas": 40, "demo": true },
            "rodadas": [{ "id": 1, "nome": "Principal", "tipo": "principal", "ordem": 0, "embaralhar": false, "formato": "folha",
                          "premios": [premio(1, "1º prêmio", 1), premio(2, "2º prêmio", 2)] }]
        });
        db::festa_set(&conn, FESTA_DEMO, &estado.to_string())?;
    }
    Ok(json!({ "ok": true }))
}


// ------------------------------------------------------------------ comandos (atalhos para as *_impl, testáveis sem janela)
#[tauri::command]
fn situacao(st: State<'_, Estado>) -> Value {
    situacao_impl(&st)
}

#[tauri::command]
async fn login(st: State<'_, Estado>, email: String, senha: String, codigo: Option<String>) -> R<Value> {
    login_impl(&st, email, senha, codigo).await
}

#[tauri::command]
async fn sair(st: State<'_, Estado>) -> R<Value> {
    sair_impl(&st).await
}

#[tauri::command]
async fn renovar(st: State<'_, Estado>) -> R<Value> {
    renovar_impl(&st).await
}

#[tauri::command]
async fn sincronizar(app: tauri::AppHandle, st: State<'_, Estado>, festa_id: i64) -> R<Value> {
    sincronizar_impl(&st, festa_id, &move |v: Value| { let _ = app.emit("progresso", v); }).await
}

#[tauri::command]
fn estado_festa(st: State<'_, Estado>, festa_id: i64) -> R<Value> {
    estado_festa_impl(&st, festa_id)
}

#[tauri::command]
fn acao(st: State<'_, Estado>, festa_id: i64, rodada_id: i64, tipo: String, dados: Value) -> R<Value> {
    acao_impl(&st, festa_id, rodada_id, tipo, dados)
}

#[tauri::command]
fn sortear(st: State<'_, Estado>, festa_id: i64, rodada_id: i64) -> R<Value> {
    sortear_impl(&st, festa_id, rodada_id)
}

#[tauri::command]
fn conferir(st: State<'_, Estado>, festa_id: i64, rodada_id: i64, codigo: String) -> R<Value> {
    conferir_impl(&st, festa_id, rodada_id, codigo)
}

#[tauri::command]
fn ranking(st: State<'_, Estado>, festa_id: i64, rodada_id: i64) -> R<Value> {
    ranking_impl(&st, festa_id, rodada_id)
}

#[tauri::command]
fn iniciar_demo(st: State<'_, Estado>, recomecar: bool) -> R<Value> {
    iniciar_demo_impl(&st, recomecar)
}


// ------------------------------------------------------------------ PDFs gerados no computador (12C)
// O servidor não guarda os arquivos: só a referência (hash, caminho, computador). Se o arquivo
// sumir ou mudar, o app avisa e gera de novo com as mesmas cartelas (o resultado é idêntico).

/// Baixa (uma vez) as artes da festa para o cache local: cabeçalho, marca d'água, coringa, logo e
/// os logos dos patrocinadores. O nome do arquivo no cache é o SHA-256 (arte trocada no painel = arquivo
/// novo; arquivo corrompido não entra). Depois copia tudo, com nomes legíveis, para a pasta da festa em
/// Documentos/Bingo Digital (o cliente vê as imagens que vão nas cartelas e no telão).
async fn baixar_artes(st: &Estado, festa: &Value, progresso: &(dyn Fn(Value) + Send + Sync)) {
    let pasta = st.pasta.join("artes");
    let _ = std::fs::create_dir_all(&pasta);
    let f = &festa["festa"];
    let imagens: Vec<&Value> = ["cabecalho", "marca_dagua", "coringa", "logo", "pix", "selo_qr"].iter().map(|c| &f["imagens"][*c])
        .chain(f["patrocinadores"].as_array().map(|v| v.iter().collect::<Vec<_>>()).unwrap_or_default())
        // cabeçalho e Pix próprios das rodadas extras
        .chain(festa["rodadas"].as_array().map(|v| v.iter().flat_map(|r| [&r["cabecalho"], &r["pix"]]).collect::<Vec<_>>()).unwrap_or_default())
        .filter(|i| i["sha256"].is_string())
        .collect();
    let total = imagens.len();
    for (n, img) in imagens.iter().enumerate() {
        progresso(json!({ "etapa": "imagens", "feitas": n, "total": total }));
        let (Some(url), Some(sha)) = (img["url"].as_str(), img["sha256"].as_str()) else { continue };
        let destino = pasta.join(sha);
        if destino.exists() {
            continue;
        }
        if let Ok(r) = st.http.get(url).send().await {
            if let Ok(bytes) = r.bytes().await {
                if hex::encode(sha2::Sha256::digest(&bytes)) == sha {
                    let _ = std::fs::write(&destino, &bytes);
                }
            }
        }
    }
    progresso(json!({ "etapa": "imagens", "feitas": total, "total": total }));
    copiar_para_documentos(st, festa);
}

/// Nome de pasta/arquivo seguro em Windows, macOS e Linux (mantém acentos e espaços).
fn nome_de_arquivo(texto: &str) -> String {
    let limpo: String = texto.chars().map(|c| if r#"/\:*?"<>|"#.contains(c) || c.is_control() { '-' } else { c }).collect();
    let limpo = limpo.trim().trim_matches('.').to_string();
    if limpo.is_empty() { "festa".into() } else { limpo.chars().take(80).collect() }
}

fn extensao(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) { "png" } else { "jpg" }
}

/// Documentos/Bingo Digital/<festa>
fn pasta_da_festa(st: &Estado, festa: &Value) -> Option<std::path::PathBuf> {
    let nome = festa["festa"]["nome"].as_str().unwrap_or("festa");
    Some(st.documentos.as_ref()?.join(nome_de_arquivo(nome)))
}

/// Copia as imagens do cache para Documentos/Bingo Digital/<festa>/Imagens e .../Patrocinadores.
/// A pasta Patrocinadores espelha o painel: patrocinador retirado no site sai daqui também.
fn copiar_para_documentos(st: &Estado, festa: &Value) {
    let Some(base) = pasta_da_festa(st, festa) else { return };
    let f = &festa["festa"];
    let imagens = base.join("Imagens");
    let _ = std::fs::create_dir_all(&imagens);
    for (chave, nome) in [("cabecalho", "Cabeçalho"), ("marca_dagua", "Marca d'água"), ("coringa", "Coringa (centro)"), ("logo", "Logo"), ("pix", "QR code do Pix")] {
        let Some(sha) = f["imagens"][chave]["sha256"].as_str() else { continue };
        if let Some(bytes) = arte_por_sha(st, sha) {
            let destino = imagens.join(format!("{nome}.{}", extensao(&bytes)));
            if std::fs::read(&destino).ok().as_deref() != Some(bytes.as_slice()) {
                let _ = std::fs::write(destino, &bytes);
            }
        }
    }
    let pasta_pat = base.join("Patrocinadores");
    let _ = std::fs::create_dir_all(&pasta_pat);
    let mut atuais = std::collections::HashSet::new();
    for (i, p) in f["patrocinadores"].as_array().cloned().unwrap_or_default().iter().enumerate() {
        let Some(bytes) = p["sha256"].as_str().and_then(|sha| arte_por_sha(st, sha)) else { continue };
        let nome = format!("{:02} - {}.{}", i + 1, nome_de_arquivo(p["nome"].as_str().unwrap_or("Patrocinador")), extensao(&bytes));
        let destino = pasta_pat.join(&nome);
        if std::fs::read(&destino).ok().as_deref() != Some(bytes.as_slice()) {
            let _ = std::fs::write(&destino, &bytes);
        }
        atuais.insert(nome);
    }
    if let Ok(lista) = std::fs::read_dir(&pasta_pat) {
        for e in lista.flatten() {
            let nome = e.file_name().to_string_lossy().into_owned();
            let nosso = nome.len() > 5 && nome.as_bytes()[..2].iter().all(u8::is_ascii_digit) && nome[2..].starts_with(" - ");
            if nosso && !atuais.contains(&nome) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

fn arte_local(st: &Estado, festa: &Value, chave: &str) -> Option<Vec<u8>> {
    arte_por_sha(st, festa["festa"]["imagens"][chave]["sha256"].as_str()?)
}

fn arte_por_sha(st: &Estado, sha: &str) -> Option<Vec<u8>> {
    std::fs::read(st.pasta.join("artes").join(sha)).ok()
}

fn slug(texto: &str) -> String {
    let mut s = String::new();
    for c in texto.to_lowercase().chars() {
        let c = match c {
            'á' | 'à' | 'â' | 'ã' => 'a', 'é' | 'ê' => 'e', 'í' => 'i', 'ó' | 'ô' | 'õ' => 'o', 'ú' | 'ü' => 'u', 'ç' => 'c',
            c => c,
        };
        if c.is_ascii_alphanumeric() { s.push(c) } else if !s.ends_with('-') { s.push('-') }
    }
    s.trim_matches('-').to_string()
}

/// Monta o PDF de uma faixa de cartelas da rodada (mesmos dados = mesmo arquivo).
/// formato: "folha" ou "meia_folha" escolhido na hora de gerar (só vale para rodada extra; a Principal é
/// sempre folha inteira). None = o formato padrão da rodada (cadastrado no site).
fn montar_pdf(st: &Estado, festa_id: i64, rodada_id: i64, primeira: i64, ultima: i64, formato: Option<&str>) -> R<(Vec<u8>, usize, Option<String>, Value)> {
    let conn = st.db.lock().unwrap();
    let estado = ler_estado(&conn, festa_id)?;
    let mut rodada = jogo::rodada(&estado, rodada_id)?.clone();
    if let Some(f @ ("folha" | "meia_folha")) = formato {
        if rodada["tipo"] != "principal" {
            rodada["formato"] = json!(f);
        }
    }
    let cartelas = db::cartelas_ativas(&conn, festa_id, Some((primeira, ultima)));
    drop(conn);
    if cartelas.is_empty() {
        return Err("Não há cartelas ativas nessa faixa. Sincronize a festa.".into());
    }
    let premios: Vec<(i64, String)> = rodada["premios"].as_array().cloned().unwrap_or_default().iter()
        .map(|p| (p["ordem"].as_i64().unwrap_or(0), p["titulo"].as_str().unwrap_or("").to_string())).collect();
    if premios.is_empty() {
        return Err(format!("Cadastre os prêmios da rodada \"{}\" antes de gerar os PDFs.", rodada["nome"].as_str().unwrap_or("")));
    }
    let f = &estado["festa"];
    let rodape = f["rodape"].as_str().map(String::from)
        .unwrap_or_else(|| format!("{} — {}", f["nome"].as_str().unwrap_or(""), f["data"].as_str().unwrap_or("")));
    let layout_srv = &f["layout"];
    let patrocinadores = if layout_srv["patrocinadores"] == "logos" {
        f["patrocinadores"].as_array().map(|v| v.iter().filter_map(|p| arte_por_sha(st, p["sha256"].as_str()?)).collect()).unwrap_or_default()
    } else {
        Vec::new()
    };
    // cabeçalho e Pix da RODADA (a extra pode ter os seus); sem eles no JSON (servidor antigo), os da festa
    let da_rodada = |chave: &str| rodada[chave]["sha256"].as_str().and_then(|sha| arte_por_sha(st, sha));
    let cabecalho_rodada = if rodada.get("cabecalho").is_some() { da_rodada("cabecalho") } else { arte_local(st, &estado, "cabecalho") };
    let (pix_rodada, pix_texto_rodada) = if rodada.get("pix_texto").is_some() {
        (da_rodada("pix"), rodada["pix_texto"].as_str().map(String::from))
    } else {
        (arte_local(st, &estado, "pix"), f["pix_texto"].as_str().map(String::from))
    };
    let arte = cartelas_pdf::Arte {
        cabecalho: cabecalho_rodada,
        marca_dagua: arte_local(st, &estado, "marca_dagua"),
        coringa: arte_local(st, &estado, "coringa"),
        logo: arte_local(st, &estado, "logo"),
        pix: pix_rodada,
        selo_qr: arte_local(st, &estado, "selo_qr"),
        patrocinadores,
    };
    let layout = cartelas_pdf::Layout {
        marca_grande: layout_srv["marca_grande"].as_bool().unwrap_or(false),
        chancela: layout_srv["chancela"].as_bool().unwrap_or(false),
        // "nenhum" sem rodapé escrito: a folha sai sem rodapé (igual à web)
        mostrar_rodape: layout_srv["patrocinadores"] != "nenhum" || f["rodape"].as_str().is_some(),
        cabecalho_texto: (layout_srv["cabecalho"] == "texto").then(|| cartelas_pdf::CabecalhoTexto {
            titulo: layout_srv["titulo"].as_str().or(f["nome"].as_str()).unwrap_or("").to_string(),
            subtitulo: layout_srv["subtitulo"].as_str().unwrap_or("").to_string(),
            fundo: layout_srv["cor_fundo"].as_str().and_then(cartelas_pdf::cor_hex).unwrap_or((0.11, 0.31, 0.85)),
            texto: layout_srv["cor_texto"].as_str().and_then(cartelas_pdf::cor_hex).unwrap_or((1.0, 1.0, 1.0)),
        }),
        pix_texto: pix_texto_rodada,
        selo: f["selo"].is_object().then(|| cartelas_pdf::Selo {
            grande: f["selo"]["modo"] == "grande",
            texto: f["selo"]["texto"].as_str().unwrap_or("Cartelas geradas pelo Bingo Digital").to_string(),
            site: f["selo"]["site"].as_str().unwrap_or("").to_string(),
        }),
    };
    // rastreio: festa, computador (da licença) e 1ª cartela do lote; igual a cada regeração
    let dispositivo = {
        let conn = st.db.lock().unwrap();
        db::kv_get(&conn, "licenca").and_then(|l| licenca::conferir(&l).ok()).and_then(|d| d["dispositivo"]["id"].as_i64())
    };
    let rastreio = match (festa_id, dispositivo) {
        (FESTA_DEMO, _) | (_, None) => "DEMONSTRAÇÃO".to_string(),
        (_, Some(d)) => format!("ref {festa_id}-{d}-{primeira:04}"),
    };
    let ctx = cartelas_pdf::Contexto { festa_nome: f["nome"].as_str().unwrap_or(""), rodape, rodada: &rodada, premios, rastreio, layout };
    let (bytes, paginas, detalhe) = cartelas_pdf::gerar(&ctx, &arte, &cartelas);
    Ok((bytes, paginas, detalhe, rodada))
}

fn referencia_json(p: &db::PdfLocal, rodada_nome: &str) -> Value {
    json!({
        "rodada_id": p.rodada_id, "nome_arquivo": p.nome, "primeira_cartela": p.primeira, "ultima_cartela": p.ultima,
        "formato": p.formato, "detalhe": p.detalhe, "paginas": p.paginas, "tamanho_bytes": p.tamanho,
        "sha256": p.sha256, "caminho_local": p.caminho, "rodada": rodada_nome,
    })
}

/// Envia ao servidor as referências de PDFs ainda não registradas (ex.: geradas offline).
async fn enviar_pdfs_pendentes(st: &Estado, festa_id: i64, tok: &str) {
    let pendentes: Vec<db::PdfLocal> = { db::pdfs(&st.db.lock().unwrap(), festa_id).into_iter().filter(|p| p.servidor_id.is_none()).collect() };
    for p in pendentes {
        if let Ok((201, r)) = chamar(st, reqwest::Method::POST, &format!("/festas/{festa_id}/pdfs"), Some(tok), Some(referencia_json(&p, ""))).await {
            db::pdf_servidor(&st.db.lock().unwrap(), p.id, r["pdf"]["id"].as_i64());
        }
    }
}

/// Gera os PDFs da rodada em arquivos de `por_arquivo` cartelas, na pasta escolhida.
pub(crate) async fn gerar_pdfs_impl(st: &Estado, festa_id: i64, rodada_id: i64, por_arquivo: i64, pasta: String, formato: Option<String>, progresso: &(dyn Fn(Value) + Send + Sync)) -> R<Value> {
    if festa_id != FESTA_DEMO {
        licenca_valida(&st.db.lock().unwrap())?;
    }
    let por_arquivo = por_arquivo.clamp(10, 2000);
    if festa_id != FESTA_DEMO {
        let festa = { ler_estado(&st.db.lock().unwrap(), festa_id)? };
        progresso(json!({ "etapa": "imagens", "feitos": 0, "total": 1, "arquivo": "conferindo as imagens da festa" }));
        baixar_artes(st, &festa, &|_| {}).await;
    }
    let (numeros, festa_nome, rodada_nome) = {
        let conn = st.db.lock().unwrap();
        let estado = ler_estado(&conn, festa_id)?;
        let rodada = jogo::rodada(&estado, rodada_id)?;
        let numeros: Vec<i64> = db::cartelas_ativas(&conn, festa_id, jogo::faixa(rodada)).iter().map(|c| c.numero).collect();
        (numeros, estado["festa"]["nome"].as_str().unwrap_or("festa").to_string(), rodada["nome"].as_str().unwrap_or("").to_string())
    };
    if numeros.is_empty() {
        return Err("Esta rodada não tem cartelas ativas. Sincronize a festa (ou gere as cartelas).".into());
    }
    let lotes: Vec<&[i64]> = numeros.chunks(por_arquivo as usize).collect();
    let total = lotes.len();
    let mut gerados = Vec::new();
    let mut caminhos: Vec<String> = Vec::new();

    for (i, lote) in lotes.iter().enumerate() {
        let (ini, fim) = (lote[0], *lote.last().unwrap());
        progresso(json!({ "feitos": i, "total": total, "arquivo": format!("gerando as cartelas {ini:04} a {fim:04}…") }));
        let (bytes, paginas, detalhe, rodada) = montar_pdf(st, festa_id, rodada_id, ini, fim, formato.as_deref())?;
        let nome = format!("lote_{:02}_{}_{:04}-{:04}_{}.pdf", i + 1, slug(&rodada_nome), ini, fim, slug(&festa_nome));
        let caminho = std::path::Path::new(&pasta).join(&nome);
        std::fs::write(&caminho, &bytes).map_err(|e| format!("Não foi possível salvar {nome}: {e}"))?;
        let registro = db::PdfLocal {
            id: 0, festa_id, rodada_id, caminho: caminho.to_string_lossy().into_owned(), nome, primeira: ini, ultima: fim,
            formato: if rodada["formato"] == "meia_folha" && rodada["tipo"] != "principal" { "meia_folha".into() } else { "folha".into() },
            detalhe, paginas: paginas as i64, tamanho: bytes.len() as i64, sha256: hex::encode(sha2::Sha256::digest(&bytes)),
            servidor_id: None, criado_em: String::new(),
        };
        let id = { db::pdf_add(&st.db.lock().unwrap(), &registro, por_arquivo)? };
        gerados.push(id);
        caminhos.push(registro.caminho.clone());
        progresso(json!({ "feitos": i + 1, "total": total, "arquivo": registro.nome }));
    }

    // referências para a nuvem (sem internet, vão na próxima sincronização)
    if festa_id != FESTA_DEMO {
        if let Ok(tok) = { token(&st.db.lock().unwrap()) } {
            enviar_pdfs_pendentes(st, festa_id, &tok).await;
        }
    }
    Ok(json!({ "arquivos": gerados.len(), "pasta": pasta, "caminhos": caminhos }))
}

/// PDFs deste computador, conferindo se cada arquivo ainda existe e não foi alterado.
pub(crate) fn listar_pdfs_impl(st: &Estado, festa_id: i64) -> R<Value> {
    let lista = { db::pdfs(&st.db.lock().unwrap(), festa_id) };
    Ok(json!(lista.iter().map(|p| {
        let situacao = match std::fs::read(&p.caminho) {
            Err(_) => "sumiu",
            Ok(b) if hex::encode(sha2::Sha256::digest(&b)) != p.sha256 => "alterado",
            Ok(_) => "ok",
        };
        json!({
            "id": p.id, "rodada_id": p.rodada_id, "nome": p.nome, "caminho": p.caminho, "primeira": p.primeira, "ultima": p.ultima,
            "formato": p.formato, "detalhe": p.detalhe, "paginas": p.paginas, "tamanho": p.tamanho, "situacao": situacao,
            "na_nuvem": p.servidor_id.is_some(), "criado_em": p.criado_em,
        })
    }).collect::<Vec<_>>()))
}

/// Gera de novo um arquivo que sumiu ou foi alterado (mesmas cartelas, mesmo caminho).
pub(crate) fn regerar_pdf_impl(st: &Estado, id: i64) -> R<Value> {
    let registro = {
        let conn = st.db.lock().unwrap();
        let festa_id: i64 = conn.query_row("SELECT festa_id FROM pdfs WHERE id = ?1", [id], |r| r.get(0)).map_err(|_| "Arquivo não encontrado.")?;
        db::pdfs(&conn, festa_id).into_iter().find(|p| p.id == id).ok_or("Arquivo não encontrado.")?
    };
    // mesmo formato do arquivo original: gerar de novo dá o mesmo PDF
    let (bytes, paginas, _, _) = montar_pdf(st, registro.festa_id, registro.rodada_id, registro.primeira, registro.ultima, Some(&registro.formato))?;
    if let Some(dir) = std::path::Path::new(&registro.caminho).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(&registro.caminho, &bytes).map_err(|e| format!("Não foi possível salvar: {e}"))?;
    let sha = hex::encode(sha2::Sha256::digest(&bytes));
    db::pdf_atualizar(&st.db.lock().unwrap(), id, &sha, bytes.len() as i64, paginas as i64);
    Ok(json!({ "ok": true, "igual_ao_original": sha == registro.sha256 }))
}

/// Remove a referência (o cliente apagou o arquivo de propósito).
pub(crate) async fn esquecer_pdf_impl(st: &Estado, id: i64) -> R<Value> {
    let (festa_id, servidor_id) = {
        let conn = st.db.lock().unwrap();
        conn.query_row("SELECT festa_id, servidor_id FROM pdfs WHERE id = ?1", [id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?)))
            .map_err(|_| "Arquivo não encontrado.")?
    };
    if let (Some(sid), Ok(tok)) = (servidor_id, { token(&st.db.lock().unwrap()) }) {
        let _ = chamar(st, reqwest::Method::DELETE, &format!("/festas/{festa_id}/pdfs/{sid}"), Some(&tok), None).await;
    }
    db::pdf_del(&st.db.lock().unwrap(), id);
    Ok(json!({ "ok": true }))
}

/// Pede ao servidor mais cartelas (os números nascem no servidor: é o que impede fraude).
pub(crate) async fn gerar_cartelas_impl(st: &Estado, festa_id: i64, quantidade: i64) -> R<Value> {
    let tok = { token(&st.db.lock().unwrap())? };
    let (status, dados) = chamar(st, reqwest::Method::POST, &format!("/festas/{festa_id}/cartelas/gerar"), Some(&tok), Some(json!({ "quantidade": quantidade }))).await?;
    if status != 200 {
        return Err(tratar_erro(st, status, &dados));
    }
    Ok(dados)
}

#[tauri::command]
async fn gerar_pdfs(app: tauri::AppHandle, st: State<'_, Estado>, festa_id: i64, rodada_id: i64, por_arquivo: i64, pasta: String, formato: Option<String>) -> R<Value> {
    gerar_pdfs_impl(&st, festa_id, rodada_id, por_arquivo, pasta, formato, &move |v: Value| { let _ = app.emit("progresso_pdf", v); }).await
}

/// Patrocinadores da festa para o carrossel do telão (imagens do cache, como data URL: funciona offline).
#[tauri::command]
fn patrocinadores(st: State<'_, Estado>, festa_id: i64) -> R<Value> {
    use base64::Engine;
    let festa = { ler_estado(&st.db.lock().unwrap(), festa_id)? };
    let lista: Vec<Value> = festa["festa"]["patrocinadores"].as_array().cloned().unwrap_or_default().iter().filter_map(|p| {
        let bytes = arte_por_sha(&st, p["sha256"].as_str()?)?;
        let tipo = if extensao(&bytes) == "png" { "image/png" } else { "image/jpeg" };
        Some(json!({ "nome": p["nome"], "imagem": format!("data:{tipo};base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes)) }))
    }).collect();
    Ok(json!(lista))
}

/// Pasta da festa em Documentos/Bingo Digital (sugerida para salvar os PDFs). Cria se não existir.
#[tauri::command]
fn pasta_festa(st: State<'_, Estado>, festa_id: i64, sub: Option<String>) -> R<Value> {
    let festa = { ler_estado(&st.db.lock().unwrap(), festa_id)? };
    let Some(mut pasta) = pasta_da_festa(&st, &festa) else { return Ok(Value::Null) };
    if let Some(sub) = sub {
        pasta = pasta.join(nome_de_arquivo(&sub));
    }
    let _ = std::fs::create_dir_all(&pasta);
    Ok(json!(pasta.to_string_lossy()))
}

#[tauri::command]
fn listar_pdfs(st: State<'_, Estado>, festa_id: i64) -> R<Value> {
    listar_pdfs_impl(&st, festa_id)
}

#[tauri::command]
fn regerar_pdf(st: State<'_, Estado>, id: i64) -> R<Value> {
    regerar_pdf_impl(&st, id)
}

#[tauri::command]
async fn esquecer_pdf(st: State<'_, Estado>, id: i64) -> R<Value> {
    esquecer_pdf_impl(&st, id).await
}

#[tauri::command]
async fn gerar_cartelas(st: State<'_, Estado>, festa_id: i64, quantidade: i64) -> R<Value> {
    gerar_cartelas_impl(&st, festa_id, quantidade).await
}

/// Registro do sorteio (tela administrativa): linha do tempo do servidor + ações deste computador
/// que ainda não foram enviadas. Sem internet, mostra só as pendentes.
pub(crate) async fn registro_impl(st: &Estado, festa_id: i64) -> R<Value> {
    let pendentes: Vec<Value> = {
        let conn = st.db.lock().unwrap();
        db::eventos_pendentes(&conn, Some(festa_id)).into_iter()
            .filter_map(|(_, corpo)| serde_json::from_str::<Value>(&corpo).ok())
            .filter(|c| c["tipo"] != "escolher")
            .collect()
    };
    let tok = { token(&st.db.lock().unwrap())? };
    let servidor = match chamar(st, reqwest::Method::GET, &format!("/festas/{festa_id}/registro"), Some(&tok), None).await {
        Ok((200, dados)) => Some(dados),
        Ok((status, dados)) => return Err(tratar_erro(st, status, &dados)),
        Err(_) => None,
    };
    Ok(json!({ "online": servidor.is_some(), "servidor": servidor, "pendentes": pendentes }))
}

#[tauri::command]
async fn registro(st: State<'_, Estado>, festa_id: i64) -> R<Value> {
    registro_impl(&st, festa_id).await
}

// ------------------------------------------------------------------ app

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let _ = VERSAO_APP.set(app.package_info().version.to_string());
            let pasta = app.path().app_data_dir().expect("pasta de dados do app");
            let conn = db::abrir(&pasta).expect("banco local");
            let http = cliente_http().expect("cliente HTTP");
            let documentos = app.path().document_dir().ok().map(|d| d.join("Bingo Digital"));
            app.manage(Estado { db: Mutex::new(conn), http, pasta, app_hash: hash_do_executavel(), documentos });
            app.manage(atualizacao::Atualizacao::default());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            situacao, login, sair, renovar, sincronizar, estado_festa, acao, sortear, conferir, ranking, iniciar_demo,
            gerar_pdfs, listar_pdfs, regerar_pdf, esquecer_pdf, gerar_cartelas, registro,
            patrocinadores, pasta_festa,
            atualizacao::atualizacao_procurar, atualizacao::atualizacao_baixar, atualizacao::atualizacao_instalar
        ])
        .build(tauri::generate_context!())
        .expect("erro ao iniciar o Bingo Digital")
        .run(|app, evento| {
            // atualização baixada e não instalada ("depois"): instala quando o app encerra
            // (fechar a janela, Cmd+Q, Alt+F4, sair pelo menu: todos passam por aqui)
            if let tauri::RunEvent::Exit = evento {
                atualizacao::instalar_ao_fechar(app);
            }
        });
}

// ------------------------------------------------------------------ testes de ponta a ponta
// Rodam contra o servidor de desenvolvimento (precisa dele no ar e da conta cliente@teste.local):
//   cargo test --lib -- --ignored --nocapture
#[cfg(test)]
mod pasta_documentos {
    use super::*;

    /// Imagens da festa copiadas do cache para Documentos/Bingo Digital/<festa> com nomes legíveis;
    /// patrocinador retirado no painel sai da pasta; arquivos do cliente na pasta não são tocados.
    #[test]
    fn copia_imagens_e_patrocinadores_com_nomes_legiveis() {
        let pasta = std::env::temp_dir().join(format!("bingo-docs-{}", uuid::Uuid::new_v4()));
        let st = Estado { db: Mutex::new(db::abrir(&pasta).unwrap()), http: reqwest::Client::new(), pasta: pasta.clone(),
            app_hash: String::new(), documentos: Some(pasta.join("Documentos").join("Bingo Digital")) };
        let cache = pasta.join("artes");
        std::fs::create_dir_all(&cache).unwrap();
        let png = |cor: u8| { let mut b = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]; b.push(cor); b };
        let guardar = |b: Vec<u8>| { let sha = hex::encode(sha2::Sha256::digest(&b)); std::fs::write(cache.join(&sha), b).unwrap(); sha };
        let (logo, p1, p2) = (guardar(png(1)), guardar(png(2)), guardar(vec![0xff, 0xd8, 3]));

        let festa = |pats: Value| json!({ "festa": { "nome": "Festa do Divino: 2027", "imagens": { "logo": { "sha256": logo } }, "patrocinadores": pats } });
        copiar_para_documentos(&st, &festa(json!([{ "nome": "Mercado Bom Preço", "sha256": p1 }, { "nome": "Padaria/Pão", "sha256": p2 }])));

        let base = pasta.join("Documentos/Bingo Digital/Festa do Divino- 2027");
        assert!(base.join("Imagens/Logo.png").exists());
        assert!(base.join("Patrocinadores/01 - Mercado Bom Preço.png").exists());
        assert!(base.join("Patrocinadores/02 - Padaria-Pão.jpg").exists());
        std::fs::write(base.join("Patrocinadores/minha anotação.txt"), "do cliente").unwrap();

        // a padaria saiu no painel: sai da pasta também (o arquivo do cliente fica)
        copiar_para_documentos(&st, &festa(json!([{ "nome": "Mercado Bom Preço", "sha256": p1 }])));
        assert!(base.join("Patrocinadores/01 - Mercado Bom Preço.png").exists());
        assert!(!base.join("Patrocinadores/02 - Padaria-Pão.jpg").exists());
        assert!(base.join("Patrocinadores/minha anotação.txt").exists());
        let _ = std::fs::remove_dir_all(&pasta);
    }
}

#[cfg(test)]
mod ponta_a_ponta {
    use super::*;

    fn estado_temporario() -> Estado {
        let pasta = std::env::temp_dir().join(format!("bingo-teste-{}", uuid::Uuid::new_v4()));
        Estado { db: Mutex::new(db::abrir(&pasta).unwrap()), http: reqwest::Client::new(), pasta: pasta.clone(), app_hash: hash_do_executavel(), documentos: Some(pasta.join("Documentos")) }
    }

    #[tokio::test]
    #[ignore]
    async fn login_baixa_sorteia_offline_e_sincroniza() {
        let st = estado_temporario();
        // BINGO_E2E_EMAIL: outra conta de teste (padrão cliente@teste.local, senha 12345678)
        let email = std::env::var("BINGO_E2E_EMAIL").unwrap_or_else(|_| "cliente@teste.local".into());
        login_impl(&st, email, "12345678".into(), None).await.expect("login");
        let sit = situacao_impl(&st);
        assert_eq!(sit["logado"], true);
        // a festa da conta com mais cartelas (a conta pode ter festas novas ainda vazias)
        let festa_id = sit["licenca"]["festas"].as_array().unwrap().iter()
            .max_by_key(|f| f["total_cartelas"].as_i64().unwrap_or(0)).unwrap()["id"].as_i64().unwrap();

        let r = sincronizar_impl(&st, festa_id, &|_| {}).await.expect("baixar");
        println!("baixou {} cartelas", r["cartelas"]);
        let estado = estado_festa_impl(&st, festa_id).unwrap();
        let rodada_id = estado["rodadas"][0]["id"].as_i64().unwrap();

        // "offline": quadro limpo, prêmio no telão (opcional), pedras da cartela 0001; tudo vai para a fila
        if !estado["rodadas"][0]["pedras"].as_array().unwrap().is_empty() {
            acao_impl(&st, festa_id, rodada_id, "limpar".into(), json!({})).unwrap();
        }
        let ultimo = estado["rodadas"][0]["premios"].as_array().unwrap().last().unwrap()["id"].as_i64().unwrap();
        acao_impl(&st, festa_id, rodada_id, "escolher".into(), json!({ "premio_id": ultimo })).unwrap();
        let cartela = db::cartela_get(&st.db.lock().unwrap(), festa_id, 1).unwrap();
        let verificador = cartela.verificador.clone().expect("cartela baixada com código verificador");
        for n in cartela.numeros.iter().flatten().flatten() {
            acao_impl(&st, festa_id, rodada_id, "pedra".into(), json!({ "numero": n })).unwrap();
        }
        let por_numero = conferir_impl(&st, festa_id, rodada_id, "1".into()).unwrap();
        let por_codigo = conferir_impl(&st, festa_id, rodada_id, verificador.to_lowercase()).unwrap();
        assert_eq!(por_numero["bateu_na_pedra"], 24);
        assert_eq!(por_codigo["codigo"], "0001", "acha pelo código verificador");
        assert_eq!(por_codigo["verificador"], verificador.as_str());
        acao_impl(&st, festa_id, rodada_id, "limpar".into(), json!({})).unwrap();
        let pendentes = estado_festa_impl(&st, festa_id).unwrap()["pendentes"].as_i64().unwrap();
        assert!(pendentes >= 26);

        // regra local igual à do servidor: sem pedra, nada a remover
        assert!(acao_impl(&st, festa_id, rodada_id, "remover".into(), json!({ "numero": 2 })).is_err());

        // volta a internet: envia tudo; o servidor guarda o quadro limpo no registro
        let r = sincronizar_impl(&st, festa_id, &|_| {}).await.expect("enviar");
        assert_eq!(r["enviados"].as_i64(), Some(pendentes), "{r}");
        assert!(r["recusado"].is_null());
        let estado = estado_festa_impl(&st, festa_id).unwrap();
        assert_eq!(estado["rodadas"][0]["premio_atual_id"].as_i64(), Some(ultimo), "o servidor guardou o prêmio do telão");
        assert!(estado["rodadas"][0]["pedras"].as_array().unwrap().is_empty());
        assert_eq!(estado["pendentes"], 0);
        let reg = registro_impl(&st, festa_id).await.unwrap();
        assert!(reg["servidor"]["rodadas"][0]["eventos"].as_array().unwrap().iter().any(|e| e["titulo"] == "Quadro limpo"));

        sair_impl(&st).await.unwrap();
        assert_eq!(situacao_impl(&st)["logado"], false);
    }

    #[tokio::test]
    #[ignore]
    async fn gera_pdfs_no_computador_e_regera_identico() {
        let st = estado_temporario();
        let email = std::env::var("BINGO_E2E_EMAIL").unwrap_or_else(|_| "cliente@teste.local".into());
        login_impl(&st, email, "12345678".into(), None).await.expect("login");
        let festa_id = situacao_impl(&st)["licenca"]["festas"][0]["id"].as_i64().unwrap();
        sincronizar_impl(&st, festa_id, &|_| {}).await.expect("baixar");
        let estado = estado_festa_impl(&st, festa_id).unwrap();
        let principal = estado["rodadas"][0]["id"].as_i64().unwrap();

        let pasta = std::env::temp_dir().join(format!("bingo-pdfs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&pasta).unwrap();
        let t = std::time::Instant::now();
        let r = gerar_pdfs_impl(&st, festa_id, principal, 1000, pasta.to_string_lossy().into(), None, &|_| {}).await.expect("gerar");
        println!("{} arquivos em {:?} na pasta {}", r["arquivos"], t.elapsed(), pasta.display());

        let lista = listar_pdfs_impl(&st, festa_id).unwrap();
        let primeiro = lista[0].clone();
        assert!(lista.as_array().unwrap().iter().all(|p| p["situacao"] == "ok" && p["na_nuvem"] == true), "{lista}");

        std::fs::remove_file(primeiro["caminho"].as_str().unwrap()).unwrap();
        assert_eq!(listar_pdfs_impl(&st, festa_id).unwrap()[0]["situacao"], "sumiu");
        let r = regerar_pdf_impl(&st, primeiro["id"].as_i64().unwrap()).unwrap();
        assert_eq!(r["igual_ao_original"], true, "gerado de novo = idêntico");
        assert_eq!(listar_pdfs_impl(&st, festa_id).unwrap()[0]["situacao"], "ok");

        // o servidor tem as referências (computador e caminho)
        let tok = token(&st.db.lock().unwrap()).unwrap();
        let (_, nuvem) = chamar(&st, reqwest::Method::GET, &format!("/festas/{festa_id}/pdfs"), Some(&tok), None).await.unwrap();
        assert!(nuvem["pdfs"].as_array().unwrap().iter().any(|p| p["sha256"] == primeiro["sha256"] || p["nome_arquivo"] == primeiro["nome"]));
        sair_impl(&st).await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn fixacao_de_certificado_so_aceita_lets_encrypt() {
        let c = cliente_com(true, true).unwrap();
        assert!(c.get("https://letsencrypt.org").send().await.is_ok(), "certificado Let's Encrypt: aceita");
        assert!(c.get("https://www.google.com").send().await.is_err(), "outro emissor (Google Trust Services): recusa");
        assert!(c.get("http://example.com").send().await.is_err(), "http puro: recusa");
    }

    #[tokio::test]
    #[ignore]
    async fn relogio_voltado_para_tras_apaga_os_dados() {
        let st = estado_temporario();
        login_impl(&st, "cliente@teste.local".into(), "12345678".into(), None).await.expect("login");
        {
            let conn = st.db.lock().unwrap();
            db::kv_set(&conn, "maior_horario", &(agora() + Duration::days(3)).to_rfc3339()).unwrap();
        }
        let sit = situacao_impl(&st);
        assert_eq!(sit["logado"], false);
        assert_eq!(sit["motivo"]["codigo"], "relogio");
        assert!(db::kv_get(&st.db.lock().unwrap(), "token").is_none(), "token apagado");
        // limpeza: o computador de teste fica revogado no servidor? não — o token sumiu localmente;
        // o registro do computador é removido pelo script de limpeza do teste.
    }
}
