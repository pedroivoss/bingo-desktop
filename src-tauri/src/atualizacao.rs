//! Atualização automática do app.
//!
//! O app pergunta ao servidor do Bingo Digital (não ao GitHub) se há versão nova:
//!   GET {API}/atualizacao/{{target}}/{{arch}}/{{current_version}}  → 204 (nada) ou JSON do updater do Tauri.
//! Quem publica as versões é o dono, no painel (Admin → Versões do app).
//!
//! Fluxo, para não atrapalhar quem está no meio de uma festa:
//! 1. procura ao abrir e depois de cada sincronização;
//! 2. baixa em segundo plano (o arquivo é conferido com a assinatura do Bingo Digital: arquivo sem a
//!    assinatura da nossa chave é recusado);
//! 3. pergunta se pode reiniciar; se o operador disser "depois", instala sozinho ao fechar o app.
//! Versão obrigatória (o servidor manda "obrigatoria": true): o app não segue sem atualizar.

use serde_json::{json, Value};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};

type R<T> = Result<T, String>;

#[derive(Default)]
pub struct Atualizacao {
    pendente: Mutex<Option<Update>>,
    baixada: Mutex<Option<(String, Vec<u8>)>>, // (versão, arquivo já conferido)
}

fn endereco() -> String {
    format!("{}/atualizacao/{{{{target}}}}/{{{{arch}}}}/{{{{current_version}}}}", crate::base_url())
}

/// Procura versão nova. null = já está na mais nova (ou sem internet: segue normal).
#[tauri::command]
pub async fn atualizacao_procurar(app: AppHandle, st: State<'_, Atualizacao>) -> R<Value> {
    let url = endereco().parse().map_err(|e| format!("{e}"))?;
    let atualizador = app
        .updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;

    let Some(nova) = atualizador.check().await.map_err(|e| e.to_string())? else {
        *st.pendente.lock().unwrap() = None;
        return Ok(Value::Null);
    };
    let ja_baixada = st.baixada.lock().unwrap().as_ref().is_some_and(|(v, _)| *v == nova.version);
    let resposta = json!({
        "versao": nova.version,
        "atual": nova.current_version,
        "notas": nova.body,
        "obrigatoria": nova.raw_json.get("obrigatoria").and_then(Value::as_bool).unwrap_or(false),
        "baixada": ja_baixada,
    });
    *st.pendente.lock().unwrap() = Some(nova);
    Ok(resposta)
}

/// Baixa (e confere a assinatura) da versão encontrada. Progresso no evento "atualizacao-progresso".
#[tauri::command]
pub async fn atualizacao_baixar(app: AppHandle, st: State<'_, Atualizacao>) -> R<()> {
    let nova = st.pendente.lock().unwrap().clone().ok_or("Nenhuma atualização encontrada.")?;
    if st.baixada.lock().unwrap().as_ref().is_some_and(|(v, _)| *v == nova.version) {
        return Ok(());
    }
    let mut baixado: u64 = 0;
    let bytes = nova
        .download(
            |pedaco, total| {
                baixado += pedaco as u64;
                let _ = app.emit("atualizacao-progresso", json!({ "baixado": baixado, "total": total }));
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    *st.baixada.lock().unwrap() = Some((nova.version.clone(), bytes));
    Ok(())
}

/// Instala a versão baixada e reinicia o app (no Windows o instalador fecha e reabre o app sozinho).
#[tauri::command]
pub fn atualizacao_instalar(app: AppHandle, st: State<'_, Atualizacao>) -> R<()> {
    instalar(&st)?;
    app.restart();
}

fn instalar(st: &Atualizacao) -> R<()> {
    let nova = st.pendente.lock().unwrap().clone().ok_or("Nenhuma atualização encontrada.")?;
    let baixada = st.baixada.lock().unwrap().take().ok_or("A atualização ainda não foi baixada.")?;
    nova.install(&baixada.1).map_err(|e| e.to_string())
}

/// Ao fechar o app: se havia uma atualização baixada e o operador escolheu "depois", instala agora.
pub fn instalar_ao_fechar(app: &AppHandle) {
    if let Some(st) = app.try_state::<Atualizacao>() {
        if st.baixada.lock().unwrap().is_some() {
            let _ = instalar(&st);
        }
    }
}
