# ModelTrace provenance

- Source: https://github.com/xqy2006/ModelTrace
- Source revision: `3f0dd2f4b451ad424f3b165a108a468efe4d4d81`
- License: MIT; see `LICENSE` in this directory.
- Imported assets: the unified fingerprint bank and an adapted TypeScript implementation of the browser scorer.
- `unified-bank.json` SHA-256: `6a3f7e4d703990a2322cf535020309380ca858654345d8fe5db278578568bad3`
- Scope: closed-set similarity attribution only. A model absent from the bank is still assigned to the nearest listed candidate.
- Calibration limits inherited from the source: results are not calibrated for same-context or multilingual comparisons.
