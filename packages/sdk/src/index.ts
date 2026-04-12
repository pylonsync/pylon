export type QueryMode = "static" | "server" | "live";

export interface AgentdbRouteDefinition {
  path: string;
  mode: QueryMode;
}

export function defineRoute(route: AgentdbRouteDefinition): AgentdbRouteDefinition {
  return route;
}

