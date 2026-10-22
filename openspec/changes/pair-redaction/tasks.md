## 1. Engine (branch, not enabled)

- [x] 1.1 `Rule::Pair` (id `pair`); `Finding.pair_basis`; module `pair` with the name lexicon (token-matched), the sensitivity flags, the value floor, and the four text shapes.
- [x] 1.2 `spans()` adds pair spans; every occurrence of a detected value is replaced and a repeat counts once.
- [x] 1.3 Structured JSON: an object with a name-like and a value-like field is judged in the JSON walk (redact and dry run).
- [x] 1.4 Dry run reports `basis=` per hit and totals per basis.
- [x] 1.5 Tests: the 5 planted fakes redacted and the 5 negative controls untouched (fixtures built from parts), plus token matching (`PATH` is not `PAT`).

## 2. Sizing (before any enable)

- [x] 2.1 Read-only archive dry run with `--rule pair` from a branch build: messages per basis, client-data paths by count only, to manager-lab and security.
- [x] 2.2 manager-lab decides what ships ((b) first if (a) is far noisier); release and swap are a separate step. Shipped (a)+(b) as cchv-v0.29.0 (manager-lab a7311a0a); live 2026-10-22T11:42:01Z with `pair` in the redact list; live canary PASS, 17 of 17.
