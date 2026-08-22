import type { NotFoundProps } from "@pylonsync/react";
import { NotFoundView } from "@pylon-cloud/ui/components/not-found-view";

// app/not-found.tsx → rendered at HTTP 404 for any unmatched URL.
//
// Before this, an unknown path answered `{"error":{"code":"NOT_FOUND"}}` from
// the API router: a correct status with a body that tells a visitor nothing
// and an agent less. This boundary is also what a markdown request for a
// missing page returns (the runtime converts it and answers 404 with
// text/markdown), so the links here are how an agent that guessed a URL wrong
// finds its way back.
export default function NotFound(_props: NotFoundProps) {
	return <NotFoundView />;
}
