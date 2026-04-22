//! TypeScript function runtime for statecraft.
//!
//! This crate provides the Rust side of the bidirectional protocol between
//! the statecraft runtime (Rust) and user-defined TypeScript functions (Bun/Deno).
//!
//! # Architecture
//!
//! ```text
//! Client ──► Rust Runtime ──► FunctionRunner ──► Bun Process
//!                │                                    │
//!                │  1. Acquire lock (mutations)        │
//!                │  2. Send {call, fn, args, auth}     │
//!                │  3. Receive {db, op, ...}      ◄────┘
//!                │  4. Execute SQL, return result  ────►│
//!                │  5. Receive {stream, data}     ◄────┘
//!                │  6. Forward SSE to client            │
//!                │  7. Receive {return, value}    ◄────┘
//!                │  8. COMMIT or ROLLBACK               │
//!                └─────────────────────────────────────-┘
//! ```
//!
//! # Function types
//!
//! - **Query**: read-only, uses read pool, concurrent execution
//! - **Mutation**: read+write, transactional, handler IS the transaction
//! - **Action**: external I/O allowed, non-transactional, calls queries/mutations

pub mod protocol;
pub mod registry;
pub mod runner;
pub mod trace;
