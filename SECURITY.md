# Security policy

## Reporting a vulnerability

Do not open a public issue for a suspected security vulnerability. Report it privately to the repository owner with a concise description, affected component, reproduction steps, impact, and any suggested mitigation.

Do not include secrets, private keys, access tokens, or personal data in reports. The maintainer will acknowledge the report, assess impact, and coordinate a fix before public disclosure.

## Local model safety

Local model artifacts are untrusted inputs. Keep scans restricted to approved directories and do not execute, deserialize, or load a discovered artifact solely because it appears in the local registry.
