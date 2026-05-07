import { query, v } from "@pylonsync/functions";

/**
 * Last 200 messages in a room, oldest first (chat-conventional —
 * scroll up for history). For a real chat app, paginate via cursor;
 * the scaffold caps at 200 since rendering more in a single FlatList
 * / SwiftUI List without virtualization gets expensive.
 */
export default query({
	args: { roomId: v.id("Room") },
	async handler(ctx, args: { roomId: string }) {
		const rows = (await ctx.db.query("Message", {
			roomId: args.roomId,
		})) as any[];
		const sorted = [...rows].sort(
			(a, b) => Date.parse(a.createdAt) - Date.parse(b.createdAt),
		);
		return sorted.slice(-200);
	},
});
