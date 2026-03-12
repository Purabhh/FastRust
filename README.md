# rust_squared

`rust_squared` is an early foundation for a FastAPI-style Rust web framework built directly on Hyper.

This workspace currently includes a working `rust_squared` crate with routing, request/response
primitives, app state, typed `Path` / `Query` / `Json` / `State` extractors, route macros, basic
middleware, and a server entrypoint with `/openapi.json` and `/docs`, plus a placeholder `cargo rsq`
CLI.
