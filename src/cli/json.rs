use std::io::Write;

// Re-exported so the out-of-scope `store` commands keep wrapping bodies in the
// shared envelope by the `json::` path. The `pointbreak.review-*` commands now call
// the `pointbreak::documents` builders directly. `EventWriteDocument` lives in
// `pointbreak::documents` alongside it but has no remaining CLI-side caller.
pub(super) use pointbreak::documents::DiagnosticDocument;

pub(super) fn write_json<T: serde::Serialize>(
    stdout: &mut dyn Write,
    document: &T,
    pretty: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if pretty {
        serde_json::to_writer_pretty(&mut *stdout, document)?;
    } else {
        serde_json::to_writer(&mut *stdout, document)?;
    }
    writeln!(stdout)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn write_json_respects_pretty_flag() {
        #[derive(serde::Serialize)]
        struct Doc {
            schema: &'static str,
            version: u32,
        }

        let mut compact = Vec::new();
        super::write_json(
            &mut compact,
            &Doc {
                schema: "test",
                version: 1,
            },
            false,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(compact).unwrap(),
            "{\"schema\":\"test\",\"version\":1}\n"
        );

        let mut pretty = Vec::new();
        super::write_json(
            &mut pretty,
            &Doc {
                schema: "test",
                version: 1,
            },
            true,
        )
        .unwrap();
        let pretty = String::from_utf8(pretty).unwrap();
        assert!(pretty.contains("\n  \"schema\""));
    }
}
