# Investigations

Working notes from debugging and research: what was tried, what was found, and what is still
unknown. These are not specifications and not user documentation. They may describe behaviour that
has since changed, so check the code before relying on them.

| Document | Covers |
|---|---|
| [`README-JUNIPER-VSRXV3.0.md`](README-JUNIPER-VSRXV3.0.md) | Juniper vSRX 3.0 boot failures on KVM: PML, SMBIOS entry point, CPU features. |
| [`private-bridges.md`](private-bridges.md) | `[[bridges]]` defects: container members not attached, interface renaming, redeploy, destroy (#345). |
| [`talos-linux.md`](talos-linux.md) | Running Talos Linux and Kubernetes clusters in labs: image, NoCloud config delivery, multi-document configs, PKI, bootstrap (#326). |

## Adding a document

- One file per investigation, named after the subject.
- Record the symptom, the evidence, what was ruled out, and the current status.
- When an investigation produces a lasting fact, move that fact to `docs/architecture/` or to
  the `sherpa-docs` repository and leave a pointer here.
- Add a row to the table above.
