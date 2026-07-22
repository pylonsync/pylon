# A2L HVAC Charge Calculator Product Spec

## Summary

Build a field-first mobile utility for HVAC technicians working with A2L refrigerants, especially R-454B and R-32. The app provides offline pressure-temperature charts, superheat and subcooling calculators, charge-limit helpers, job history, and branded service reports.

App Store Optimization (ASO) is the primary acquisition channel.

## Product Positioning

**Name:** A2L HVAC Charge Calculator

**One-liner:** Field-ready R-454B and R-32 charge calculators, PT charts, and service reports for HVAC techs.

**Primary promise:** No login, no signal required, fast answers on the roof or at the unit.

**App Store subtitle:** R-454B, R-32 PT Chart

## Target User

### Primary Persona

Commercial and residential HVAC technicians who need quick refrigerant references and charge calculations while servicing or commissioning equipment.

### Secondary Persona

Small HVAC company owners who want branded job reports, customer equipment history, and shared records across technicians.

## Problem

HVAC techs are navigating the A2L refrigerant transition while still servicing older systems. They need fast, trusted, offline access to:

- R-454B, R-32, and R-410A PT data
- Superheat and subcooling calculations
- Dew point and bubble point references
- Charge-limit and safety helpers
- Job notes and service records
- Customer-facing reports

Existing tools are often broad, legacy, login-gated, cluttered, or not specific to the A2L transition.

## Goals

- Rank for high-intent App Store searches around A2L, R-454B, R-32, PT charts, superheat, and subcooling.
- Deliver a useful free app without onboarding friction.
- Convert active techs to Pro for saved history, reports, branding, photos, and team sync.
- Showcase Pylon's mobile sync, file storage, auth, functions, background jobs, and offline-first behavior.

## Non-Goals

- Replace manufacturer charging procedures.
- Provide legal or safety certification.
- Support every refrigerant in the first release.
- Build a full HVAC field-service platform.
- Require account creation for the core calculator experience.

## MVP Scope

### Free Features

- Offline PT charts for R-454B, R-32, and R-410A.
- Refrigerant selector with recent refrigerants pinned.
- Superheat calculator.
- Subcooling calculator.
- Dew point and bubble point helper for blended refrigerants.
- Unit conversion between Fahrenheit/Celsius and PSI/kPa.
- Basic safety reference screen for A2L handling.
- Local-only recent calculations.

### Pro Features

- Unlimited saved jobs.
- Customer and equipment history.
- Job notes and photo attachments.
- Branded PDF service reports.
- Company logo and contact profile.
- Cloud sync across devices.
- Team workspace with shared job records.

### V1.1 Candidates

- Barcode/QR scan for equipment tags.
- Offline manufacturer note templates.
- Leak check workflow.
- Commissioning checklist.
- Push reminders for follow-up visits.
- Export CSV for office/admin use.

## Key User Flows

### Quick Calculation

1. User opens app.
2. App lands directly on last-used refrigerant and calculator.
3. User enters pressure and temperature readings.
4. App returns superheat/subcooling result.
5. User can save the calculation to a job or discard it.

### Job Report

1. User creates or selects customer.
2. User creates job with equipment details.
3. User records refrigerant, readings, notes, and photos.
4. App generates branded PDF report.
5. User shares report by email, text, or system share sheet.

### Team Sync

1. Owner creates company workspace.
2. Owner invites technicians.
3. Technicians save jobs offline in the field.
4. Pylon syncs job records, photos, and reports when online.
5. Owner sees completed jobs in near real time.

## Data Model

### Entities

- `User`
  - `id`
  - `email`
  - `name`
  - `role`
  - `companyId`

- `Company`
  - `id`
  - `name`
  - `logoFileId`
  - `phone`
  - `email`
  - `address`
  - `plan`

- `Customer`
  - `id`
  - `companyId`
  - `name`
  - `phone`
  - `email`
  - `address`

- `Equipment`
  - `id`
  - `companyId`
  - `customerId`
  - `manufacturer`
  - `model`
  - `serialNumber`
  - `refrigerant`
  - `notes`

- `Job`
  - `id`
  - `companyId`
  - `customerId`
  - `equipmentId`
  - `technicianId`
  - `status`
  - `startedAt`
  - `completedAt`
  - `location`

- `Reading`
  - `id`
  - `jobId`
  - `refrigerant`
  - `calculationType`
  - `pressure`
  - `lineTemperature`
  - `ambientTemperature`
  - `targetValue`
  - `actualValue`
  - `unitSystem`

- `JobPhoto`
  - `id`
  - `jobId`
  - `fileId`
  - `caption`
  - `createdAt`

- `Report`
  - `id`
  - `jobId`
  - `fileId`
  - `generatedAt`
  - `sharedAt`

- `Refrigerant`
  - `id`
  - `name`
  - `type`
  - `safetyClass`
  - `ptDataVersion`

## Pylon Architecture Fit

### Mobile Client

- Swift or React Native client.
- Local-first calculator and job history.
- Offline mutation queue for jobs, readings, and photos.
- File upload queue for job photos and logos.

### Backend

- Pylon schema for users, companies, jobs, customers, equipment, readings, photos, and reports.
- Auth for Pro and team workspaces.
- RBAC for owner, admin, and technician roles.
- File storage for logos, photos, and generated PDFs.
- Server functions for report generation and share links.
- Background jobs for PDF rendering and cleanup.
- Realtime sync for team job updates.

### Offline Behavior

- Calculator data ships with the app and requires no network.
- Saved jobs write locally first.
- Photos queue until network returns.
- Conflicts are resolved by record ownership and latest local edit timestamp for draft fields.
- Completed reports are regenerated server-side after sync if needed.

## Monetization

### Free

- Core PT charts.
- Superheat and subcooling calculators.
- Recent unsynced calculations.

### Pro Individual

- `$9.99/mo` or `$49.99/yr`
- Saved job history.
- Photo attachments.
- PDF reports.
- Company branding.

### Team

- `$29/mo` base, includes 3 users.
- Additional users at `$5/user/mo`.
- Shared job history.
- Admin dashboard.
- Team report branding.

## ASO Strategy

### Primary Keywords

- `A2L HVAC`
- `R454B PT chart`
- `R-454B calculator`
- `R32 PT chart`
- `superheat calculator`
- `subcooling calculator`
- `HVAC charge calculator`
- `refrigerant PT chart`

### App Store Listing

**App name:** A2L HVAC Charge Calculator

**Subtitle:** R-454B, R-32 PT Chart

**Keyword field candidates:** `hvac,a2l,r454b,r32,pt chart,superheat,subcooling,refrigerant,charge calculator`

### Screenshot Concepts

1. R-454B charge calculator.
2. Works offline on the roof.
3. Dew and bubble point built in.
4. Save job readings and photos.
5. Branded service reports.

## Success Metrics

### Acquisition

- App Store keyword ranking for `R454B PT chart`, `A2L HVAC`, and `superheat calculator`.
- App Store product page conversion rate.
- Install volume by keyword cluster.

### Activation

- First calculation completed.
- Refrigerant selected.
- Second calculation within 7 days.
- First saved job.

### Revenue

- Free-to-Pro conversion rate.
- Trial-to-paid conversion rate.
- Monthly recurring revenue.
- Annual plan conversion rate.
- Team workspace creation rate.

### Retention

- Weekly active calculators.
- Jobs saved per active user.
- Reports generated per Pro user.
- Month-1 paid retention.

## MVP Build Plan

1. Define refrigerant/PT data format and calculator formulas.
2. Build offline calculator UI.
3. Add local recent calculations.
4. Add Pylon schema for Pro job history.
5. Add auth and paid entitlement checks.
6. Add customer, equipment, job, and reading CRUD.
7. Add photo upload.
8. Add branded report generation.
9. Add App Store listing assets and metadata.
10. Ship TestFlight and validate with HVAC techs before public release.

## Risks

- Incorrect refrigerant calculations could damage trust quickly.
- Broad HVAC calculator keywords may be competitive.
- Some technicians may prefer free web charts unless reporting/history creates enough value.
- App Store screenshots and title need to make the A2L/R-454B wedge obvious.
- Formula and PT data sources need validation before release.

## Open Questions

- Which mobile stack should be used first: Swift or React Native?
- Which refrigerants are mandatory for launch beyond R-454B, R-32, and R-410A?
- What is the authoritative PT data source and licensing status?
- Should Pro billing use App Store subscriptions only, or support web checkout for teams?
- Should PDF generation happen on-device, server-side, or both?
