import { createPylonServer } from "@pylonsync/next/server";

export const pylon = createPylonServer({
	cookieName: "__APP_NAME_SNAKE___session",
});
