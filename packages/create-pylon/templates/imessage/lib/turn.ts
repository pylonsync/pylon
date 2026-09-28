import type { ActionCtx, AgentToolRun } from "@pylonsync/functions";

export interface Turn {
  conversationId: string;
  contactId: string;
}

const NOT_ANSWERING = "This tool is only available while answering a text.";

/**
 * Which contact a tool call is about. Every assistant tool starts here.
 *
 * processTurn runs the agent with ctx.agents.run and puts the conversation
 * and turn in the run's context; only server code can set it. resolveTurn
 * maps that to exactly one contact, and only while that turn is running.
 * Nothing the model or the texter writes can change the mapping.
 */
export async function currentTurn(ctx: Pick<ActionCtx, "runQuery">, run: AgentToolRun): Promise<Turn> {
  const conversationId = run.context?.conversationId;
  const turnId = run.context?.turnId;
  if (typeof conversationId !== "string" || typeof turnId !== "string") throw new Error(NOT_ANSWERING);
  const turn = await ctx.runQuery<Turn | null>("resolveTurn", { conversationId, turnId });
  if (!turn) throw new Error(NOT_ANSWERING);
  return turn;
}
