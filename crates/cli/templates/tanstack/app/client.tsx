/// <reference types="vinxi/types/client" />
import { hydrateRoot } from "react-dom/client";
import { StartClient } from "@tanstack/react-start";
import { configureClient } from "@pylonsync/react";
import { createRouter } from "./router";
import { PYLON_URL } from "./lib/pylon";

// Wire up the SDK before hydrating so any hooks that mount immediately
// see the configured baseUrl.
configureClient({ baseUrl: PYLON_URL });

const router = createRouter();

hydrateRoot(document, <StartClient router={router} />);
