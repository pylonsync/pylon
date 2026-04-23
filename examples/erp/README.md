# Pylon ERP — Custom Fabrication

A multi-tenant B2B ERP for custom door / window / cabinet shops.
Demonstrates organizations, roles, inventory, orders, and policy-gated
data access in a single-file app.

## Quick start

```bash
# From the repo root:
cargo build --release -p pylon-cli

# Terminal 1: dev server
cd examples/erp
../../target/release/pylon dev

# Terminal 2: web UI
cd examples/erp/web
bun install
bun dev
# → http://localhost:5174
```

## What's in it

- **Orgs + roles** — `Organization` + `OrgMember` with roles
  (`owner / admin / estimator / production / viewer`). Users can belong
  to multiple orgs; the active one lives on the session.
- **Policies** — every org-scoped entity uses the new row-level policy
  DSL (`allowRead`, `allowUpdate`, `allowDelete`) to gate on
  `data.orgId == auth.tenantId`. The raw `/api/entities/*` endpoints
  enforce the same rules without going through a function.
- **Inventory ledger** — `Material` + append-only `StockMovement` so you
  can reconcile physical counts without losing history.
- **Orders + line items** — `createOrder` writes header + lines in one
  mutation, computes totals server-side, and supports a simple status
  state machine via `advanceOrderStatus`.

## Extending

Good starter exercises:

- Add a `ProductOption` configurator UI (schema already supports it).
- Convert accepted quotes into orders in one click (stub function
  `acceptQuote` in `functions/` if you want).
- Hook the low-stock dashboard card into a purchase-order function.
- Build a per-line `ProductionTask` board for the shop floor.
