//! Licença assinada pelo servidor (Ed25519). O app só CONFERE com a chave pública embutida:
//! não existe segredo no executável que permita forjar uma licença.
//! Formato: base64url(json) + "." + base64url(assinatura de 64 bytes sobre o json).

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::Value;

/// Chave pública do servidor de desenvolvimento. O build de produção recebe a do servidor real
/// pela variável de ambiente BINGO_CHAVE_PUBLICA (no GitHub Actions).
const CHAVE_PUBLICA_DEV: &str = "bIcqRLnCAlQSB5ev3QR5QiyM8Y1CLjCNjQAOWJjaYco=";

pub fn chave_publica() -> &'static str {
    option_env!("BINGO_CHAVE_PUBLICA").unwrap_or(CHAVE_PUBLICA_DEV)
}

/// Confere a assinatura e devolve os dados da licença. Qualquer alteração no texto invalida.
pub fn conferir(licenca: &str) -> Result<Value, String> {
    let (json_b64, assinatura_b64) = licenca.split_once('.').ok_or("Licença em formato inválido.")?;
    let json = URL_SAFE_NO_PAD.decode(json_b64).map_err(|_| "Licença em formato inválido.")?;
    let assinatura = URL_SAFE_NO_PAD.decode(assinatura_b64).map_err(|_| "Licença em formato inválido.")?;

    let chave: [u8; 32] = STANDARD
        .decode(chave_publica())
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or("Chave pública do app inválida.")?;
    let verificador = VerifyingKey::from_bytes(&chave).map_err(|_| "Chave pública do app inválida.")?;
    let assinatura: [u8; 64] = assinatura.try_into().map_err(|_| "Assinatura da licença inválida.")?;

    verificador
        .verify(&json, &Signature::from_bytes(&assinatura))
        .map_err(|_| "Assinatura da licença não confere.".to_string())?;

    serde_json::from_slice(&json).map_err(|_| "Licença em formato inválido.".into())
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn licenca_adulterada_nao_confere() {
        assert!(conferir("abc.def").is_err());
        assert!(conferir("sem-ponto").is_err());
    }
}
