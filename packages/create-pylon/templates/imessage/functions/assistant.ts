import { agent, v, type ActionCtx } from "@pylonsync/functions";
import { ASSISTANT_MODEL, SYSTEM_PROMPT } from "../lib/assistant";

interface Turn {
  conversationId: string;
  contactId: string;
}

// Which contact is this generation about? Every tool starts here. The agent
// loop records its stream id on the AgentRun row it is working on, and
// processTurn created that run with the conversation id as its title, so
// resolveTurn can map this call back to exactly one contact. Nothing the
// model or the texter writes can change that mapping.
async function currentTurn(ctx: ActionCtx): Promise<Turn> {
  const streamId = ctx.stream?.id;
  if (!streamId) throw new Error("This tool is only available while answering a text.");
  const turn = await ctx.runQuery<Turn | null>("resolveTurn", { streamId });
  if (!turn) throw new Error("This tool is only available while answering a text.");
  return turn;
}

// The assistant. Pylon's agent() runs the tool loop, persists every turn into
// AgentRun/AgentMessage (live in the dashboard), and streams tokens.
//
// auth "admin": only the owner's session may run it. processTurn calls it as
// the owner for each inbound text (lib/agent-call.ts); any other account gets
// FORBIDDEN before the model is called.
//
// The tools act only on the contact of the current turn and never take a
// contact, phone number, or id from the model.
export default agent({
  auth: "admin",
  model: ASSISTANT_MODEL,
  maxSteps: 8,
  maxTokens: 1024,
  timeout: 240,
  system: SYSTEM_PROMPT,
  tools: {
    remember_note: {
      description:
        "Save a short fact about the person you are texting (a preference, a name, a date). One fact per call.",
      args: { text: v.string() },
      handler: async (ctx, input) => {
        const turn = await currentTurn(ctx);
        return ctx.runMutation("toolSaveNote", { contactId: turn.contactId, text: String(input.text) });
      },
    },
    recall_notes: {
      description: "List the facts saved about the person you are texting, newest first.",
      handler: async (ctx) => {
        const turn = await currentTurn(ctx);
        return ctx.runQuery("toolListNotes", { contactId: turn.contactId });
      },
    },
    forget_note: {
      description: "Delete one saved fact about the person you are texting, by the id recall_notes returned.",
      args: { noteId: v.string() },
      handler: async (ctx, input) => {
        const turn = await currentTurn(ctx);
        return ctx.runMutation("toolForgetNote", { contactId: turn.contactId, noteId: String(input.noteId) });
      },
    },
    set_reminder: {
      description:
        "Schedule a text to the person you are texting at a specific time. `when` is ISO 8601 with a UTC offset, e.g. 2026-10-03T09:00:00-05:00. `text` is the exact message they will receive.",
      args: { when: v.string(), text: v.string() },
      handler: async (ctx, input) => {
        const turn = await currentTurn(ctx);
        return ctx.runMutation("toolSetReminder", {
          contactId: turn.contactId,
          conversationId: turn.conversationId,
          when: String(input.when),
          text: String(input.text),
        });
      },
    },
    list_reminders: {
      description: "List the upcoming reminders for the person you are texting.",
      handler: async (ctx) => {
        const turn = await currentTurn(ctx);
        return ctx.runQuery("toolListReminders", { contactId: turn.contactId });
      },
    },
    cancel_reminder: {
      description: "Cancel one upcoming reminder for the person you are texting, by the id list_reminders returned.",
      args: { reminderId: v.string() },
      handler: async (ctx, input) => {
        const turn = await currentTurn(ctx);
        return ctx.runMutation("toolCancelReminder", {
          contactId: turn.contactId,
          reminderId: String(input.reminderId),
        });
      },
    },
  },
});
