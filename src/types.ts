export type Instance = {
  name: string;
  claw_type: string;
  display_name: string;
  logo: string;
  sandbox_type: string;
  version: string;
  gateway_port: number;
  ttyd_port: number;
};

export type ClawType = {
  id: string;
  display_name: string;
  logo: string;
  package_manager: "npm" | "pip";
  npm_package: string;
  pip_package: string;
  default_port: number;
  supports_mcp: boolean;
  supports_browser: boolean;
  has_gateway_ui: boolean;
  supports_native: boolean;
};

export type UpgradeInfo = {
  instance: string;
  current: string;
  latest: string;
  security: boolean;
};

export type UpgradeProgress = {
  message: string;
  percent: number;
  stage: string;
};
