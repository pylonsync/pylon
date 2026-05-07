import { ChatRoom } from "./components/ChatRoom";

// All data flows through `db.useQuery` in the client component below —
// no server-side initial fetch needed. The sync engine handles initial
// hydration + every subsequent change over the same WebSocket.
export default function HomePage() {
	return (
		<main className="h-screen flex">
			<ChatRoom />
		</main>
	);
}
