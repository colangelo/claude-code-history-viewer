## Why

ac/security F7 (P1, #11; security review `docs/reviews/2026-10-22-cchv-structured-secrets.md`): on
2026-10-22 a session printed about 45 real secret values from a CI variable snapshot into a tool result, and
the detector flagged none. The values sat in name/value structures (`"Name": "DB_PASSWORD", "Value": "…"`, or
XML `Name=`/`Value=`), where no single field matches `prefix`, `pem`, `bearer` or `assign`. Agents print such
structured stores routinely (CI variables, `docker inspect`, cloud CLI JSON, manifests, `vault kv get -format=json`).

## What Changes

- A new rule `pair`. It recognises a name-like field (`name`, `key`, `variable`, `id`) and a value-like field
  (`value`, `secret`, `data`, `val`) in the same object, in four text shapes (flat JSON objects, including
  escaped JSON; XML `Name=`/`Value=` attributes on one element; XML `<Name>`/`<Value>` child elements; YAML
  `name:`/`value:` blocks) and in structured JSON values.
- It fires when (a) the name is secret-named (the `assign` lexicon plus PASS, PASSWD, PWD, PAT, AUTH, APIKEY,
  ACCESS_KEY, CLIENT_SECRET, CONN(ECTION)_STRING, DSN, SAS, SIGNING_KEY and a trailing `KEY` token, matched by
  token so `PATH` is not `PAT`), or (b) the same object carries a sensitivity flag (`Sensitive`, `IsSensitive`,
  `Secure`, `Masked`, `Protected`, `Encrypted` = true).
- Value floor: at least 8 characters; not a whole-value reference or placeholder; not a boolean; not all digits
  under 12; not a URL without userinfo; not an existing mask.
- Redaction uses the fixed marker `[REDACTED:pair]` and replaces every occurrence of a detected value in the same
  text; a value repeated in one text counts once.
- A `Finding` records the basis (`name`, `flag` or `name+flag`), so the dry run can size (a) against (b).

## Impact

- Engine and dry run only in this change. Not enabled for redaction until manager-lab has seen the archive
  counts; if (a) is far noisier than (b), (b) ships first. Note that a hub binary carrying the rule FLAGS `pair`
  findings at ingest even when `hub.toml` does not redact it, so the release question includes the daily count.
