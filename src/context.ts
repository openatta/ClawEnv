import { createContext, useContext } from "solid-js";
import type { Accessor } from "solid-js";
import type { Instance, ClawType } from "./types";

export type AppContextType = {
  instances: Accessor<Instance[]>;
  healths: Accessor<Record<string, string>>;
  clawTypes: Accessor<ClawType[]>;
  refreshInstances: () => void;
  openInstallWindow: (clawType?: string) => void;
};

export const AppContext = createContext<AppContextType>();

export function useApp(): AppContextType {
  const ctx = useContext(AppContext);
  if (!ctx) throw new Error("useApp must be used within AppContext.Provider");
  return ctx;
}
