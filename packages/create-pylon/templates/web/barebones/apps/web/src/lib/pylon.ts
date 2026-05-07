import { createPylonServer } from "@pylonsync/next/server";

/**
 * Single server-helper instance. Imported by every Server Component
 * and Server Action that needs to talk to the Pylon control plane.
 *
 * `cookieName` MUST match the backend's emitted cookie. Pylon uses
 * `${app_name}_session` from the manifest — for this app that's
 * `__APP_NAME_SNAKE___session`. Pin it in code (NOT env) so a bad
 * deployment env can't silently break auth.
 */
export const pylon = createPylonServer({
	cookieName: "__APP_NAME_SNAKE___session",
});
