import { action, v } from "@pylonsync/functions";
import { siteConfig } from "../lib/site.config";
import { isAllowedModel, modelAllowedFor, type LlmTurn } from "../lib/chat";

// Generate the assistant's reply to the latest message in a conversation.
//
// Call it with `streamFn("respond", …)`: text streams back as it generates.
// Every function stream is resumable, and `_beginTurn` stores this call's
// stream id on the assistant message, so another tab (or this tab after a
// reload) attaches to the same reply with `resumeStream(streamId)`. The final
// text is written to the message row, which syncs to every open tab.
//
// Guests can call it. `_beginTurn` enforces the hourly limits from
// lib/site.config.ts and allows one generation per person at a time; the model
// must be one listed there, and guests are limited to `guestModels`.
export default action({
  auth: "guest",
  // Streaming does not extend the call deadline; long replies need the room.
  timeout: 300,
  args: {
    conversationId: v.string(),
    content: v.optional(v.string()),
    regenerate: v.optional(v.boolean()),
    model: v.string(),
  },
  async handler(ctx, args) {
    // A refused turn (limit, busy, bad input) resolves with its code instead
    // of throwing: a thrown code does not survive `streamFn`, and the client
    // branches on it.
    if (!isAllowedModel(args.model)) {
      return { status: "rejected" as const, errorCode: "MODEL_NOT_ALLOWED", message: "That model is not available." };
    }
    if (!modelAllowedFor(args.model, ctx.auth.isGuest)) {
      return { status: "rejected" as const, errorCode: "MODEL_NEEDS_ACCOUNT", message: "Sign in to use that model." };
    }

    let turn: { messageId: string; history: LlmTurn[] };
    try {
      turn = await ctx.runMutation("_beginTurn", {
        conversationId: args.conversationId,
        content: args.content,
        regenerate: args.regenerate ?? false,
        model: args.model,
        streamId: ctx.stream.id,
      });
    } catch (err) {
      const e = err as { code?: string; message?: string };
      return { status: "rejected" as const, errorCode: e.code ?? "INTERNAL", message: e.code ? (e.message ?? "") : "" };
    }

    let streamed = "";
    try {
      const res = await ctx.llm.stream(
        {
          system: siteConfig.chat.systemPrompt,
          messages: turn.history,
          model: args.model,
          max_tokens: siteConfig.chat.maxOutputTokens,
        },
        (e) => {
          if (e.type === "text_delta") {
            streamed += e.text;
            ctx.stream.write(e.text);
          }
        },
      );
      const text = res.content
        .filter((b): b is { type: "text"; text: string } => b.type === "text")
        .map((b) => b.text)
        .join("");
      await ctx.runMutation("_finishTurn", {
        messageId: turn.messageId,
        content: text || streamed,
        status: "done",
      });
      return { messageId: turn.messageId, status: "done" as const };
    } catch (err) {
      const errorCode = (err as { code?: string }).code ?? "LLM_FAILED";
      await ctx.runMutation("_finishTurn", {
        messageId: turn.messageId,
        content: streamed,
        status: "error",
        errorCode,
      });
      return { messageId: turn.messageId, status: "error" as const, errorCode };
    }
  },
});
