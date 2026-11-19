# __APP_NAME__

Scaffolded by [@pylonsync/create-pylon](https://npmjs.com/@pylonsync/create-pylon).

## Layout

```
apps/
  api/         Pylon backend — schema, policies, function handlers
  ios/         Swift / SwiftUI app (if you picked --platforms ios)
  expo/        Expo + React Native app (if you picked --platforms expo)
```

## Getting started

```sh
# Install
bun install   # or pnpm install / yarn / npm install

# Run everything (the API + every frontend you picked)
bun run dev
```

- **api:** http://localhost:4321, Pylon control plane
- **expo** runs Metro on a separate port (if scaffolded)
- **mobile** lives in `apps/mobile/`; open it in Xcode or run `swift run`

## What to do next

- Edit `apps/api/schema.ts` to add entities + policies.
- Drop handlers into `apps/api/functions/` — auto-discovered by name.
- For iOS: SwiftUI views in `apps/ios/Sources/__APP_NAME_PASCAL__/`.
- For expo: screens in `apps/expo/src/screens/`.

## Docs

[docs.pylonsync.com](https://docs.pylonsync.com)
