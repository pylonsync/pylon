import { pylon } from "@/lib/pylon";
import { ChatRoom } from "./components/ChatRoom";

export const dynamic = "force-dynamic";

type Room = {
	id: string;
	slug: string;
	name: string;
	createdAt: string;
};

type Message = {
	id: string;
	roomId: string;
	authorId: string;
	authorName: string;
	body: string;
	createdAt: string;
};

export default async function HomePage() {
	const rooms = await pylon
		.json<Room[]>("/api/fn/listRooms", {
			method: "POST",
			body: "{}",
			headers: { "Content-Type": "application/json" },
		})
		.catch(() => [] as Room[]);

	const initialRoom = rooms[0] ?? null;
	const initialMessages = initialRoom
		? await pylon
				.json<Message[]>("/api/fn/roomMessages", {
					method: "POST",
					body: JSON.stringify({ roomId: initialRoom.id }),
					headers: { "Content-Type": "application/json" },
				})
				.catch(() => [] as Message[])
		: [];

	return (
		<main className="h-screen flex">
			<ChatRoom
				initialRooms={rooms}
				initialActiveRoom={initialRoom}
				initialMessages={initialMessages}
			/>
		</main>
	);
}
