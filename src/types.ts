export type Instance = {
  name: string;
  sandbox_type: string;
  version: string;
  gateway_port: number;
  ttyd_port: number;
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
