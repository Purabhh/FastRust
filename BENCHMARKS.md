# Benchmarks

This workspace now includes a baseline benchmark harness built with Criterion.

## Goals

- Measure route lookup overhead inside the trie router
- Measure end-to-end in-process request dispatch through `RsqApp::handle`
- Keep benchmark inputs stable so future changes are comparable

## Included benches

- `cargo bench -p rust_squared --bench routing_bench`
  - `router_lookup_static`
  - `router_lookup_param`
- `cargo bench -p rust_squared --bench throughput_bench`
  - `app_handle_static_route`
  - `app_handle_param_route`

## What this harness tells you

- Whether router changes slow down path resolution
- Whether endpoint ergonomics add meaningful overhead to request dispatch
- Whether regressions are coming from matching logic or the higher request pipeline

## Next layer to add

For realistic throughput numbers, compare the same tiny API in:

- raw Hyper
- FastRust
- Axum
- FastAPI + Uvicorn

Then drive them with a load tool such as `oha` or `wrk` using the same payload, concurrency, and duration.

## Record every benchmark run

When you start collecting published numbers, pin:

- CPU model
- RAM
- Windows version or Linux distro
- Rust version (`rustc --version`)
- Benchmark command
- Date of run
- Payload and concurrency settings
