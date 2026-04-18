import { createContext, useContext } from "solid-js";
import type { Accessor } from "solid-js";
import type { Instance, ClawType } from "./types";

export type AppContextType = {
  instances: Accessor<Instance[]>;
  healths: Accessor<Record<string, string>>;
  clawTypes: Accessor<ClawType[]>;
  refreshInstances: () => void;
  openInstallWindow: (clawType?: string) => void;
  /**
   * Monotonic counter bumped whenever a cached gateway token should be
   * considered stale (e.g. after a start/restart regenerates the token file).
   * Pages that cache tokens watch this via createEffect and refetch.
   */
  tokenEpoch: Accessor<number>;
};

export const AppContext = createContext<AppContextType>();

export function useApp(): AppContextType {
  const ctx = useContext(AppContext);
  if (!ctx) throw new Error("useApp must be used within AppContext.Provider");
  return ctx;
}
