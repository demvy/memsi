use std::collections::BTreeMap;

/// Read literal dotenv assignments; never execute shell or interpolate secrets.
/// Process environment takes precedence, including an explicitly empty value.
pub fn local_env(key: &str) -> Option<String> {
    std::env::var(key).ok().or_else(|| dotenv().remove(key))
}

pub(crate) fn dotenv() -> BTreeMap<String, String> {
    let Ok(input) = std::fs::read_to_string(".env") else {
        return BTreeMap::new();
    };

    parse_dotenv(&input)
}

fn parse_dotenv(input: &str) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for line in input.lines() {
        let line = line.trim().strip_prefix("export ").unwrap_or(line.trim());
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        let key = key.trim();
        if key.is_empty() || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            continue;
        }

        let value = value.trim();
        let value = if value.starts_with(['\'', '"']) {
            if value.len() < 2 || value.as_bytes().last() != value.as_bytes().first() {
                continue;
            }

            &value[1..value.len() - 1]
        } else {
            value.split(" #").next().unwrap_or(value).trim()
        };

        values.insert(key.into(), value.into());
    }

    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotenv_parses_literals_comments_export_quotes_and_duplicate_keys() {
        let values = parse_dotenv(
            "\n# comment\nexport A = first\n A=last # comment\nB='a # b'\nC=\"$(never execute) $A\"\nEMPTY=\nBAD-KEY=no\n=empty-key\nNO_EQUALS\nBAD_QUOTE='open\nSHORT='\nMISMATCH=\"value'\n",
        );
        assert_eq!(
            values,
            BTreeMap::from([
                ("A".into(), "last".into()),
                ("B".into(), "a # b".into()),
                ("C".into(), "$(never execute) $A".into()),
                ("EMPTY".into(), "".into()),
            ])
        );
    }
}
