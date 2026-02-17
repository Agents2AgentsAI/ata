export type AtaConfigValue = string | number | boolean | AtaConfigValue[] | AtaConfigObject;

export type AtaConfigObject = { [key: string]: AtaConfigValue };

export type AtaOptions = {
  ataPathOverride?: string;
  baseUrl?: string;
  apiKey?: string;
  /**
   * Additional `--config key=value` overrides to pass to the Ata CLI.
   *
   * Provide a JSON object and the SDK will flatten it into dotted paths and
   * serialize values as TOML literals so they are compatible with the CLI's
   * `--config` parsing.
   */
  config?: AtaConfigObject;
  /**
   * Environment variables passed to the Ata CLI process. When provided, the SDK
   * will not inherit variables from `process.env`.
   */
  env?: Record<string, string>;
};
