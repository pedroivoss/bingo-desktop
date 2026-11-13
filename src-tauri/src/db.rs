//! Banco local (SQLite, arquivo na pasta de dados do app). Guarda o que foi sincronizado
//! (licença, festas, cartelas) e a fila de eventos do sorteio ainda não enviados.
//! Os PDFs gerados NÃO ficam aqui: são arquivos do cliente, na pasta que ele escolher.

use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub struct Cartela {
    pub codigo: String,
    pub numero: i64,
    pub numeros: Vec<Vec<Option<i64>>>, // colunas B, I, N, G, O
    pub cancelada: bool,
    pub motivo: Option<String>,
    pub verificador: Option<String>, // código de 5 caracteres impresso embaixo do número
}

pub fn abrir(pasta: &Path) -> Result<Connection, String> {
    std::fs::create_dir_all(pasta).map_err(|e| e.to_string())?;
    let conn = Connection::open(pasta.join("bingo.db")).map_err(|e| e.to_string())?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         CREATE TABLE IF NOT EXISTS kv (chave TEXT PRIMARY KEY, valor TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS festas (id INTEGER PRIMARY KEY, dados TEXT NOT NULL, sincronizada_em TEXT);
         CREATE TABLE IF NOT EXISTS cartelas (
             festa_id INTEGER NOT NULL, codigo TEXT NOT NULL, numero INTEGER NOT NULL,
             numeros TEXT NOT NULL, cancelada INTEGER NOT NULL DEFAULT 0, motivo TEXT,
             PRIMARY KEY (festa_id, codigo));
         CREATE TABLE IF NOT EXISTS eventos (
             uuid TEXT PRIMARY KEY, festa_id INTEGER NOT NULL, corpo TEXT NOT NULL,
             criado_em TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'pendente', mensagem TEXT);
         CREATE TABLE IF NOT EXISTS pdfs (
             id INTEGER PRIMARY KEY AUTOINCREMENT, festa_id INTEGER NOT NULL, rodada_id INTEGER NOT NULL,
             caminho TEXT NOT NULL, nome TEXT NOT NULL, primeira INTEGER NOT NULL, ultima INTEGER NOT NULL,
             por_arquivo INTEGER NOT NULL, formato TEXT NOT NULL, detalhe TEXT, paginas INTEGER NOT NULL,
             tamanho INTEGER NOT NULL, sha256 TEXT NOT NULL, servidor_id INTEGER, criado_em TEXT NOT NULL);",
    )
    .map_err(|e| e.to_string())?;
    // versões antigas do app: coluna do código verificador (as cartelas são baixadas de novo)
    let tem_verificador: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('cartelas') WHERE name = 'verificador'")
        .and_then(|mut st| st.exists([]))
        .unwrap_or(false);
    if !tem_verificador {
        conn.execute_batch("ALTER TABLE cartelas ADD COLUMN verificador TEXT;").map_err(|e| e.to_string())?;
    }
    Ok(conn)
}

/// Festa baixada antes do código verificador existir: precisa baixar as cartelas de novo.
pub fn faltam_verificadores(conn: &Connection, festa_id: i64) -> bool {
    conn.query_row("SELECT EXISTS(SELECT 1 FROM cartelas WHERE festa_id = ?1 AND verificador IS NULL)", [festa_id], |r| r.get::<_, i64>(0))
        .map(|v| v == 1)
        .unwrap_or(false)
}

pub fn kv_get(conn: &Connection, chave: &str) -> Option<String> {
    conn.query_row("SELECT valor FROM kv WHERE chave = ?1", [chave], |r| r.get(0))
        .optional()
        .ok()
        .flatten()
}

pub fn kv_set(conn: &Connection, chave: &str, valor: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO kv (chave, valor) VALUES (?1, ?2) ON CONFLICT(chave) DO UPDATE SET valor = excluded.valor",
        params![chave, valor],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}


/// Apaga tudo o que foi sincronizado (licença vencida, conta bloqueada ou logout).
/// Mantém o identificador do computador e o maior horário já visto (proteção do relógio).
pub fn apagar_dados_da_conta(conn: &Connection) {
    let _ = conn.execute_batch(
        "DELETE FROM festas WHERE id <> 0; DELETE FROM cartelas WHERE festa_id <> 0; DELETE FROM eventos; DELETE FROM pdfs WHERE festa_id <> 0;
         DELETE FROM kv WHERE chave NOT IN ('uuid', 'maior_horario') AND chave NOT LIKE 'demo%';",
    );
}

pub fn festa_get(conn: &Connection, id: i64) -> Option<String> {
    conn.query_row("SELECT dados FROM festas WHERE id = ?1", [id], |r| r.get(0)).optional().ok().flatten()
}

pub fn festa_set(conn: &Connection, id: i64, dados: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO festas (id, dados, sincronizada_em) VALUES (?1, ?2, datetime('now'))
         ON CONFLICT(id) DO UPDATE SET dados = excluded.dados, sincronizada_em = excluded.sincronizada_em",
        params![id, dados],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

pub fn cartela_set(conn: &Connection, festa_id: i64, c: &Cartela) -> Result<(), String> {
    conn.execute(
        "INSERT INTO cartelas (festa_id, codigo, numero, numeros, cancelada, motivo, verificador) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(festa_id, codigo) DO UPDATE SET numeros = excluded.numeros, cancelada = excluded.cancelada, motivo = excluded.motivo,
           verificador = COALESCE(excluded.verificador, cartelas.verificador)",
        params![festa_id, c.codigo, c.numero, serde_json::to_string(&c.numeros).unwrap(), c.cancelada as i64, c.motivo, c.verificador],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

fn linha(r: &rusqlite::Row) -> rusqlite::Result<Cartela> {
    let numeros: String = r.get(2)?;
    Ok(Cartela {
        codigo: r.get(0)?,
        numero: r.get(1)?,
        numeros: serde_json::from_str(&numeros).unwrap_or_default(),
        cancelada: r.get::<_, i64>(3)? == 1,
        motivo: r.get(4)?,
        verificador: r.get(5)?,
    })
}

/// Cartela pelo código verificador ("7K3QX").
pub fn cartela_por_verificador(conn: &Connection, festa_id: i64, verificador: &str) -> Option<Cartela> {
    conn.query_row(
        "SELECT codigo, numero, numeros, cancelada, motivo, verificador FROM cartelas WHERE festa_id = ?1 AND verificador = ?2",
        params![festa_id, verificador],
        linha,
    )
    .optional()
    .ok()
    .flatten()
}

pub fn cartela_get(conn: &Connection, festa_id: i64, numero: i64) -> Option<Cartela> {
    conn.query_row(
        "SELECT codigo, numero, numeros, cancelada, motivo, verificador FROM cartelas WHERE festa_id = ?1 AND numero = ?2",
        params![festa_id, numero],
        linha,
    )
    .optional()
    .ok()
    .flatten()
}

/// Cartelas ativas de uma faixa (None = todas).
pub fn cartelas_ativas(conn: &Connection, festa_id: i64, faixa: Option<(i64, i64)>) -> Vec<Cartela> {
    let (ini, fim) = faixa.unwrap_or((0, i64::MAX));
    let mut st = match conn.prepare(
        "SELECT codigo, numero, numeros, cancelada, motivo, verificador FROM cartelas
         WHERE festa_id = ?1 AND cancelada = 0 AND numero BETWEEN ?2 AND ?3 ORDER BY numero",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    st.query_map(params![festa_id, ini, fim], linha).map(|it| it.filter_map(Result::ok).collect()).unwrap_or_default()
}

pub fn total_cartelas(conn: &Connection, festa_id: i64) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM cartelas WHERE festa_id = ?1", [festa_id], |r| r.get(0)).unwrap_or(0)
}

pub fn evento_add(conn: &Connection, festa_id: i64, uuid: &str, corpo: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO eventos (uuid, festa_id, corpo, criado_em) VALUES (?1, ?2, ?3, datetime('now'))",
        params![uuid, festa_id, corpo],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

pub fn eventos_pendentes(conn: &Connection, festa_id: Option<i64>) -> Vec<(String, String)> {
    let sql = "SELECT uuid, corpo FROM eventos WHERE status = 'pendente' AND (?1 IS NULL OR festa_id = ?1) ORDER BY rowid";
    let mut st = match conn.prepare(sql) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    st.query_map([festa_id], |r| Ok((r.get(0)?, r.get(1)?))).map(|it| it.filter_map(Result::ok).collect()).unwrap_or_default()
}

pub fn evento_status(conn: &Connection, uuid: &str, status: &str, mensagem: Option<&str>) {
    let _ = conn.execute("UPDATE eventos SET status = ?2, mensagem = ?3 WHERE uuid = ?1", params![uuid, status, mensagem]);
}

// ------------------------------------------------------------------ PDFs gerados neste computador

pub struct PdfLocal {
    pub id: i64,
    pub festa_id: i64,
    pub rodada_id: i64,
    pub caminho: String,
    pub nome: String,
    pub primeira: i64,
    pub ultima: i64,
    pub formato: String,
    pub detalhe: Option<String>,
    pub paginas: i64,
    pub tamanho: i64,
    pub sha256: String,
    pub servidor_id: Option<i64>,
    pub criado_em: String,
}

#[allow(clippy::too_many_arguments)]
pub fn pdf_add(conn: &Connection, p: &PdfLocal, por_arquivo: i64) -> Result<i64, String> {
    conn.execute(
        "INSERT INTO pdfs (festa_id, rodada_id, caminho, nome, primeira, ultima, por_arquivo, formato, detalhe, paginas, tamanho, sha256, criado_em)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, datetime('now'))",
        params![p.festa_id, p.rodada_id, p.caminho, p.nome, p.primeira, p.ultima, por_arquivo, p.formato, p.detalhe, p.paginas, p.tamanho, p.sha256],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

pub fn pdf_atualizar(conn: &Connection, id: i64, sha256: &str, tamanho: i64, paginas: i64) {
    let _ = conn.execute("UPDATE pdfs SET sha256 = ?2, tamanho = ?3, paginas = ?4 WHERE id = ?1", params![id, sha256, tamanho, paginas]);
}

pub fn pdf_servidor(conn: &Connection, id: i64, servidor_id: Option<i64>) {
    let _ = conn.execute("UPDATE pdfs SET servidor_id = ?2 WHERE id = ?1", params![id, servidor_id]);
}

pub fn pdf_del(conn: &Connection, id: i64) {
    let _ = conn.execute("DELETE FROM pdfs WHERE id = ?1", [id]);
}

pub fn pdfs(conn: &Connection, festa_id: i64) -> Vec<PdfLocal> {
    let mut st = match conn.prepare(
        "SELECT id, festa_id, rodada_id, caminho, nome, primeira, ultima, formato, detalhe, paginas, tamanho, sha256, servidor_id, criado_em
         FROM pdfs WHERE festa_id = ?1 ORDER BY id",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    st.query_map([festa_id], |r| {
        Ok(PdfLocal {
            id: r.get(0)?, festa_id: r.get(1)?, rodada_id: r.get(2)?, caminho: r.get(3)?, nome: r.get(4)?,
            primeira: r.get(5)?, ultima: r.get(6)?, formato: r.get(7)?, detalhe: r.get(8)?, paginas: r.get(9)?,
            tamanho: r.get(10)?, sha256: r.get(11)?, servidor_id: r.get(12)?, criado_em: r.get(13)?,
        })
    })
    .map(|it| it.filter_map(Result::ok).collect())
    .unwrap_or_default()
}
