# Runtime refactor does not preserve legacy compatibility by default

- Status: accepted
- Date: 2026-05-28

During the Trading Runtime refactor, documented target architecture and domain language take precedence over preserving old public APIs, event names, or compatibility constructors. Temporary compatibility layers are allowed only when explicitly documented as temporary and paired with a removal criterion, because otherwise they obscure the runtime boundary and invite future code to depend on transitional shapes instead of the intended Trading Runtime model.
