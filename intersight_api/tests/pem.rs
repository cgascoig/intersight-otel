use intersight_api::simplesigner::{Signer, SignerError};
use ring::signature::{self, KeyPair, UnparsedPublicKey};

const PEM_V2_EXAMPLE: &[u8] = include_bytes!("examples/example-v2.pem");
const PEM_V3_EXAMPLE: &[u8] = include_bytes!("examples/example-v3.pem");
const PEM_V2_EXAMPLE_TRAILING_NEWLINE: &[u8] =
    include_bytes!("examples/example-v2-trailing-newline.pem");
const PEM_V3_EXAMPLE_TRAILING_NEWLINE: &[u8] =
    include_bytes!("examples/example-v3-trailing-newline.pem");
const PEM_V2_EXAMPLE_MALFORMED: &[u8] = include_bytes!("examples/example-v2-malformed.pem");
const PEM_V3_EXAMPLE_MALFORMED: &[u8] = include_bytes!("examples/example-v3-malformed.pem");

#[test]
fn test_load_pem_v2() -> Result<(), SignerError> {
    let signer = Signer::from_pem(PEM_V2_EXAMPLE)?;
    assert!(matches!(signer, Signer::Rsa { .. }));

    verify_signature(&signer, b"123")?;

    let signer = Signer::from_pem(PEM_V2_EXAMPLE_TRAILING_NEWLINE)?;
    assert!(matches!(signer, Signer::Rsa { .. }));

    verify_signature(&signer, b"123")?;

    Ok(())
}

#[test]
fn test_load_pem_v3() -> Result<(), SignerError> {
    let signer = Signer::from_pem(PEM_V3_EXAMPLE)?;
    assert!(matches!(signer, Signer::Ecdsa { .. }));

    verify_signature(&signer, b"123")?;

    let signer = Signer::from_pem(PEM_V3_EXAMPLE_TRAILING_NEWLINE)?;
    assert!(matches!(signer, Signer::Ecdsa { .. }));

    verify_signature(&signer, b"123")?;

    Ok(())
}

#[test]
fn test_load_malformed_pem_v3() -> Result<(), SignerError> {
    let err = Signer::from_pem(PEM_V3_EXAMPLE_MALFORMED)
        .expect_err("Loading malformed key should have errored");
    assert!(matches!(err, SignerError::KeyError(..)));

    Ok(())
}

#[test]
fn test_load_malformed_pem_v2() -> Result<(), SignerError> {
    let err = Signer::from_pem(PEM_V2_EXAMPLE_MALFORMED)
        .expect_err("Loading malformed key should have errored");
    assert!(matches!(err, SignerError::KeyError(..)));

    Ok(())
}

#[test]
fn test_rejects_unsupported_pem_type() {
    let error = Signer::from_pem(b"-----BEGIN PUBLIC KEY-----\nAQID\n-----END PUBLIC KEY-----")
        .expect_err("public keys are not supported");
    assert!(matches!(error, SignerError::KeyError(message) if message == "unsupported key type"));
}

#[test]
fn test_signatures_depend_on_message() -> Result<(), SignerError> {
    let signer = Signer::from_pem(PEM_V2_EXAMPLE)?;
    let first = signer.sign_to_vec(b"first")?;
    let second = signer.sign_to_vec(b"second")?;
    assert_ne!(first, second);
    Ok(())
}

fn verify_signature(signer: &Signer, message: &[u8]) -> Result<(), SignerError> {
    let signature = signer.sign_to_vec(message)?;
    let (algorithm, public_key): (&dyn signature::VerificationAlgorithm, &[u8]) = match signer {
        Signer::Rsa(key_pair) => (
            &signature::RSA_PKCS1_2048_8192_SHA256,
            key_pair.public_key().as_ref(),
        ),
        Signer::Ecdsa(key_pair) => (
            &signature::ECDSA_P256_SHA256_ASN1,
            key_pair.public_key().as_ref(),
        ),
    };
    UnparsedPublicKey::new(algorithm, public_key)
        .verify(message, &signature)
        .map_err(|_| SignerError::KeyError("generated signature did not verify".to_string()))
}
