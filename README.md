# GraphQL Code Generator (Rust Implementation)

A high-performance Rust implementation of GraphQL Code Generator.

## Project Goal

Port and reimplement GraphQL Code Generator in Rust to achieve:
- **Better performance** through Rust's zero-cost abstractions
- **Type safety** with Rust's strong typing system
- **Memory efficiency** with Rust's ownership model
- **Native compilation** for faster execution

## Implementation Progress

### Phase 1: Core Infrastructure

- [ ] Schema parser (GraphQL SDL) — still shells out to Node for `.graphql` (native not wired yet)
- [ ] AST node types
- [x] Document parser (`.graphql` operations/fragments) — wired via `graphql-parser`

#### CLI
- [x] "init" sub-command
- [ ] default generation behavior (parity with `generate-and-save` / `codegen` still in progress)
- [ ] "verify" subcommand — *new feature*

### Phase 2: Plugin System
- [ ] Plugin trait definition
- [ ] Plugin loader
- [ ] Plugin registry
- [x] First TypeScript plugin — `plugin-typescript` (minimal; grows with parity)

### Phase 4: Optimization
- [ ] Performance benchmarks
- [ ] Memory profiling
- [ ] Caching strategies

### Upstream `packages/` parity

All paths below are under [`graphql-code-generator` → `packages/`](https://github.com/dotansimha/graphql-code-generator/tree/master/packages) (**19** npm packages).

#### Core packages

- [ ] **`graphql-codegen-cli`** (`graphql-codegen-cli/`) → `crates/graphql-codegen-rust-cli` — partial (CLI, config, generate pipeline, …)
- [ ] **`graphql-codegen-core`** (`graphql-codegen-core/`) → mostly `graphql-codegen-rust-cli` (`codegen.rs`, …); no dedicated crate yet
- [ ] **`utils/plugins-helpers`** (`@graphql-codegen/plugin-helpers`) → `crates/plugin-helpers` — partial
- [ ] **`utils/graphql-codegen-testing`** — not ported (upstream dev/test helpers)

#### Plugin packages

**Presets**

- [ ] **`presets/client`**
- [ ] **`presets/graphql-modules`**
- [ ] **`presets/swc-plugin`**

**`plugins/typescript/*`**

- [ ] **`plugins/typescript/typescript`** → `crates/plugin-typescript` — started (upstream-aligned module layout + plugin output shape; partial visitor parity)
- [ ] **`plugins/typescript/typed-document-node`** → `crates/plugin-typed-document-node` — started (dev-test `githunt/typed-document-nodes.ts` parity; upstream-style extension points added)
- [ ] **`plugins/typescript/resolvers`**
- [ ] **`plugins/typescript/operations`** → `crates/plugin-typescript-operations` — started (dev-test `githunt/types.ts` + `star-wars/types.ts` parity; config surface + visitor pipeline still partial)
- [ ] **`plugins/typescript/gql-tag-operations`**
- [ ] **`plugins/typescript/document-nodes`**

**`plugins/other/*`**

- [ ] **`plugins/other/visitor-plugin-common`** → `crates/visitor-plugin-common` — started (`utils` + client-side document-node helpers; still partial)
- [ ] **`plugins/other/add`**
- [ ] **`plugins/other/fragment-matcher`**
- [ ] **`plugins/other/introspection`**
- [ ] **`plugins/other/schema-ast`**
- [ ] **`plugins/other/time`**

Check a box when that upstream package is **fully** ported at useful parity; notes describe partial Rust work before then.

## Technology Stack

- **Language**: Rust
- **Package Manager**: Cargo
- **Linter**: clippy
- **Formatter**: rustfmt

## References

- [Original GraphQL Code Generator](https://github.com/dotansimha/graphql-code-generator)

## License

MIT