// Which certificate in the profile's root bundle is its trust anchor — the
// selection behind `identity.ca_fingerprint` (§5.1). Pure: PEM in, choice
// out. `derive` reads the two files and hands their bytes here.
//
// THE SIGNATURE IS THE ONLY TEST. A subject DN and a key identifier are names,
// and names mislead both ways: the same DN can be encoded two ways (a
// PrintableString root over a UTF8String issuer field is still the issuer),
// and a second root can carry the same DN and key identifier over another
// key. So no bundle certificate is excluded or chosen by name. The anchor is
// the one whose key verifies the top of the chain the profile presents.
//
// THE PROFILE MAY PRESENT A CHAIN. mtls.cert.pem holds the leaf and, when an
// intermediate issued it, the intermediates above it (bundle-create
// `--goat-cli-mtls-cert`, "leaf, chain"). Each certificate there must be
// signed by the next one, and the bundle root must sign the last. A root
// appended to the chain is its own top: it verifies itself.

use x509_parser::certificate::X509Certificate;

/// The bundle certificate whose key signed the top of the presented chain,
/// with its subject. `roots` are the bundle's CERTIFICATE blocks;
/// `presented` is the profile certificate file, named `presented_src` in
/// refusals. None, or more than one, is refused naming them.
pub fn issuing_root<'r>(
    roots: &[&'r pem::Pem],
    presented: &[u8],
    presented_src: &str,
) -> Result<(&'r pem::Pem, String), String> {
    let blocks = certificate_blocks(presented, presented_src)?;
    let chain = parse_chain(&blocks, presented_src)?;
    let top = chain_top(&chain, presented_src)?;

    let n = roots.len();
    let mut signers: Vec<(&'r pem::Pem, String)> = Vec::new();
    let mut subjects: Vec<String> = Vec::new();
    for root_pem in roots {
        let Ok((_, root)) = x509_parser::parse_x509_certificate(root_pem.contents()) else {
            subjects.push("<not readable X.509>".into());
            continue;
        };
        let subject = root.subject().to_string();
        subjects.push(subject.clone());
        if top.verify_signature(Some(root.public_key())).is_ok() {
            signers.push((*root_pem, subject));
        }
    }
    match signers.len() {
        1 => Ok(signers.remove(0)),
        0 => Err(format!(
            "none of the {n} certificate(s) in the CA root bundle ({}) signed the profile \
             certificate's chain (its top is {}, issued by {}). The fingerprint is the \
             counterparty's trust anchor for every peering certificate we issue, so which one \
             is meant is not a guess this tool may make",
            subjects.join("; "),
            top.subject(),
            top.issuer()
        )),
        k => {
            let named = signers
                .iter()
                .map(|(_, s)| s.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            Err(format!(
                "{k} of the {n} certificates in the CA root bundle ({named}) verify as the \
                 issuer of the profile certificate's chain; they share a key, and which one is \
                 the anchor is not a guess this tool may make"
            ))
        }
    }
}

/// The file's CERTIFICATE blocks, leaf first. Any other block is skipped, and
/// a file with none says what it does hold.
fn certificate_blocks(bytes: &[u8], src: &str) -> Result<Vec<pem::Pem>, String> {
    let items = pem::parse_many(bytes).unwrap_or_default();
    if items.is_empty() {
        return Err(format!(
            "the profile certificate at {src} is not readable PEM"
        ));
    }
    let tags: Vec<String> = items.iter().map(|p| p.tag().to_string()).collect();
    let certs: Vec<pem::Pem> = items
        .into_iter()
        .filter(|p| p.tag() == "CERTIFICATE")
        .collect();
    if certs.is_empty() {
        return Err(format!(
            "the profile certificate at {src} holds no CERTIFICATE block, only {}",
            tags.join(", ")
        ));
    }
    Ok(certs)
}

fn parse_chain<'b>(blocks: &'b [pem::Pem], src: &str) -> Result<Vec<X509Certificate<'b>>, String> {
    blocks
        .iter()
        .enumerate()
        .map(|(i, p)| {
            x509_parser::parse_x509_certificate(p.contents())
                .map(|(_, c)| c)
                .map_err(|_| format!("certificate {} in {src} is not readable X.509", i + 1))
        })
        .collect()
}

/// The last certificate of a chain in which each one is signed by the next.
fn chain_top<'c, 'b>(
    chain: &'c [X509Certificate<'b>],
    src: &str,
) -> Result<&'c X509Certificate<'b>, String> {
    for pair in chain.windows(2) {
        if pair[0]
            .verify_signature(Some(pair[1].public_key()))
            .is_err()
        {
            return Err(format!(
                "the chain in {src} is broken: {} is not signed by {}, the certificate after it",
                pair[0].subject(),
                pair[1].subject()
            ));
        }
    }
    chain
        .last()
        .ok_or_else(|| format!("the profile certificate at {src} holds no certificate"))
}

/// Real CAs and leaves minted with rcgen, for this module's tests and for
/// `derive`'s file-level ones.
#[cfg(test)]
pub(super) mod test_pki {
    use rcgen::{
        BasicConstraints, CertificateParams, DnType, DnValue, IsCa, KeyPair, PKCS_ECDSA_P256_SHA256,
    };

    pub struct Ca {
        pub cert: rcgen::Certificate,
        pub key: KeyPair,
    }

    pub fn key() -> KeyPair {
        KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap()
    }

    pub fn ca_params(cn: impl Into<DnValue>) -> CertificateParams {
        let mut params = CertificateParams::new(vec![]).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.distinguished_name.push(DnType::CommonName, cn);
        params
    }

    /// A self-signed root.
    pub fn root(cn: &str) -> Ca {
        root_from(ca_params(cn), key())
    }

    pub fn root_from(params: CertificateParams, key: KeyPair) -> Ca {
        Ca {
            cert: params.self_signed(&key).unwrap(),
            key,
        }
    }

    /// A CA certificate `parent` signed.
    pub fn intermediate(cn: &str, parent: &Ca) -> Ca {
        let key = key();
        let cert = ca_params(cn)
            .signed_by(&key, &parent.cert, &parent.key)
            .unwrap();
        Ca { cert, key }
    }

    /// A leaf `ca` signed, carrying the authority key identifier.
    pub fn leaf_signed_by(ca: &Ca) -> String {
        let mut params = CertificateParams::new(vec![]).unwrap();
        params
            .distinguished_name
            .push(DnType::CommonName, "op-alice");
        params.use_authority_key_identifier_extension = true;
        params.signed_by(&key(), &ca.cert, &ca.key).unwrap().pem()
    }
}

#[cfg(test)]
mod tests {
    use super::test_pki::*;
    use super::*;
    use rcgen::{DnValue, KeyIdMethod, KeyPair};

    /// The DER of the chosen bundle certificate, or the refusal.
    fn pick(bundle: &[pem::Pem], presented: &str) -> Result<Vec<u8>, String> {
        let refs: Vec<&pem::Pem> = bundle.iter().collect();
        issuing_root(&refs, presented.as_bytes(), "cert.pem").map(|(p, _)| p.contents().to_vec())
    }

    fn bundle(roots: &[&Ca]) -> Vec<pem::Pem> {
        roots
            .iter()
            .map(|c| pem::Pem::new("CERTIFICATE", c.cert.der().to_vec()))
            .collect()
    }

    fn der(ca: &Ca) -> Vec<u8> {
        ca.cert.der().to_vec()
    }

    #[test]
    fn one_root_that_signed_the_leaf_is_the_anchor() {
        let a = root("root-a");
        let leaf = leaf_signed_by(&a);
        assert_eq!(pick(&bundle(&[&a]), &leaf), Ok(der(&a)));
    }

    #[test]
    fn one_root_that_did_not_sign_the_leaf_is_refused() {
        // The one-root bundle used to be fingerprinted unread; a wrong single
        // root was #1289 again.
        let a = root("root-a");
        let c = root("root-c");
        let why = pick(&bundle(&[&a]), &leaf_signed_by(&c)).unwrap_err();
        assert!(why.contains("root-a") && why.contains("root-c"), "{why}");
    }

    #[test]
    fn two_roots_the_signer_is_the_anchor_wherever_it_sits() {
        let a = root("root-a");
        let b = root("root-b");
        let leaf = leaf_signed_by(&a);
        assert_eq!(pick(&bundle(&[&b, &a]), &leaf), Ok(der(&a)));
        assert_eq!(pick(&bundle(&[&a, &b]), &leaf), Ok(der(&a)));
    }

    #[test]
    fn two_roots_neither_signed_is_refused_naming_all_three() {
        let a = root("root-a");
        let b = root("root-b");
        let c = root("root-c");
        let why = pick(&bundle(&[&a, &b]), &leaf_signed_by(&c)).unwrap_err();
        assert!(
            why.contains("root-a") && why.contains("root-b") && why.contains("root-c"),
            "{why}"
        );
    }

    #[test]
    fn a_root_with_the_same_name_and_key_id_that_did_not_sign_is_not_the_anchor() {
        // Same subject DN and same subject key identifier, another key. Only
        // the signature tells them apart: drop that check and this fails.
        let same_id = || {
            let mut p = ca_params("goat-ca");
            p.key_identifier_method = KeyIdMethod::PreSpecified(vec![7; 20]);
            p
        };
        let real = root_from(same_id(), key());
        let impostor = root_from(same_id(), key());
        let leaf = leaf_signed_by(&real);
        assert_eq!(pick(&bundle(&[&impostor, &real]), &leaf), Ok(der(&real)));
        assert!(pick(&bundle(&[&impostor]), &leaf).is_err());
    }

    #[test]
    fn the_issuer_name_in_another_string_type_still_resolves() {
        // The bundle root encodes CN as PrintableString; the leaf's issuer
        // field, written from a UTF8String view of the same CA and key, holds
        // other bytes for the same name.
        let ca_key = key();
        let printable = DnValue::PrintableString("goat-ca".try_into().unwrap());
        let in_bundle = root_from(
            ca_params(printable),
            KeyPair::from_pem(&ca_key.serialize_pem()).unwrap(),
        );
        let issuer_view = root_from(ca_params("goat-ca"), ca_key);
        let leaf = leaf_signed_by(&issuer_view);

        let leaf_blk = pem::parse(&leaf).unwrap();
        let (_, parsed_leaf) = x509_parser::parse_x509_certificate(leaf_blk.contents()).unwrap();
        let (_, parsed_root) = x509_parser::parse_x509_certificate(in_bundle.cert.der()).unwrap();
        assert_ne!(
            parsed_leaf.issuer().as_raw(),
            parsed_root.subject().as_raw(),
            "the test needs the two encodings to differ"
        );

        let other = root("root-b");
        assert_eq!(
            pick(&bundle(&[&other, &in_bundle]), &leaf),
            Ok(der(&in_bundle))
        );
    }

    #[test]
    fn two_roots_sharing_a_key_are_refused_naming_both() {
        // A root reissued over the same key: both verify, and the two
        // fingerprints differ.
        let k = key();
        let old = root_from(
            ca_params("goat-ca-2025"),
            KeyPair::from_pem(&k.serialize_pem()).unwrap(),
        );
        let new = root_from(ca_params("goat-ca-2026"), k);
        let why = pick(&bundle(&[&old, &new]), &leaf_signed_by(&new)).unwrap_err();
        assert!(
            why.contains("2 of the 2")
                && why.contains("goat-ca-2025")
                && why.contains("goat-ca-2026"),
            "{why}"
        );
    }

    #[test]
    fn an_unreadable_bundle_entry_is_skipped_and_named() {
        let a = root("root-a");
        let c = root("root-c");
        let mut b = vec![pem::Pem::new("CERTIFICATE", b"not der".to_vec())];
        b.extend(bundle(&[&a]));
        assert_eq!(pick(&b, &leaf_signed_by(&a)), Ok(der(&a)));
        let why = pick(&b, &leaf_signed_by(&c)).unwrap_err();
        assert!(why.contains("<not readable X.509>"), "{why}");
    }

    #[test]
    fn a_profile_certificate_file_that_is_not_a_certificate_says_what_it_holds() {
        let b = bundle(&[&root("root-a")]);
        let why = pick(&b, "not pem at all").unwrap_err();
        assert!(why.contains("not readable PEM"), "{why}");

        let key_only = pem::encode(&pem::Pem::new("PRIVATE KEY", vec![1, 2, 3]));
        let why = pick(&b, &key_only).unwrap_err();
        assert!(
            why.contains("no CERTIFICATE block") && why.contains("PRIVATE KEY"),
            "{why}"
        );

        let garbage = pem::encode(&pem::Pem::new("CERTIFICATE", vec![1, 2, 3]));
        let why = pick(&b, &garbage).unwrap_err();
        assert!(why.contains("not readable X.509"), "{why}");
    }

    #[test]
    fn a_leaf_issued_by_an_intermediate_resolves_to_the_root_above_it() {
        let r = root("root-r");
        let int = intermediate("int-i", &r);
        let other = root("root-b");
        let chain = format!("{}{}", leaf_signed_by(&int), int.cert.pem());
        assert_eq!(pick(&bundle(&[&other, &r]), &chain), Ok(der(&r)));
        // Without the intermediate, no bundle root signed the leaf.
        assert!(pick(&bundle(&[&other, &r]), &leaf_signed_by(&int)).is_err());
    }

    #[test]
    fn a_chain_with_the_root_appended_resolves_to_that_root() {
        let r = root("root-r");
        let int = intermediate("int-i", &r);
        let chain = format!("{}{}{}", leaf_signed_by(&int), int.cert.pem(), r.cert.pem());
        assert_eq!(pick(&bundle(&[&root("root-b"), &r]), &chain), Ok(der(&r)));
    }

    #[test]
    fn a_broken_presented_chain_is_refused() {
        let r = root("root-r");
        let stray = root("stray");
        let chain = format!("{}{}", leaf_signed_by(&r), stray.cert.pem());
        let why = pick(&bundle(&[&r]), &chain).unwrap_err();
        assert!(why.contains("broken") && why.contains("stray"), "{why}");
    }
}
