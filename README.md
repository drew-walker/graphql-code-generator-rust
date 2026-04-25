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
- [ ] Schema parser (GraphQL SDL)
- [ ] AST node types
- [ ] Document parser
- [ ] CLI
    - [ ] "init" sub-command
    - [ ] default generation behavior
    - [ ] "verify" sub-comand *NEW FEATURE 

### Phase 2: Plugin System
- [ ] Plugin trait definition
- [ ] Plugin loader
- [ ] Plugin registry
- [ ] First TypeScript plugin

### Phase 4: Optimization
- [ ] Performance benchmarks
- [ ] Memory profiling
- [ ] Caching strategies

## Technology Stack

- **Language**: Rust
- **Package Manager**: Cargo
- **Linter**: clippy
- **Formatter**: rustfmt

## References

- [Original GraphQL Code Generator](https://github.com/dotansimha/graphql-code-generator)

## License

MIT