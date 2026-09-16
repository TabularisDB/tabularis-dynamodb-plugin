// AWS regions offered by the plugin's per-connection region selector.
//
// Must stay in sync with the `region` plugin setting's `options` in the
// repository's `.tabularium` — `ui/test/regions.test.ts` reads that file and
// fails if the two lists drift.
export const AWS_REGIONS = [
  "us-east-1",
  "us-east-2",
  "us-west-1",
  "us-west-2",
  "af-south-1",
  "ap-east-1",
  "ap-east-2",
  "ap-south-1",
  "ap-south-2",
  "ap-southeast-1",
  "ap-southeast-2",
  "ap-southeast-3",
  "ap-southeast-4",
  "ap-southeast-5",
  "ap-southeast-6",
  "ap-southeast-7",
  "ap-northeast-1",
  "ap-northeast-2",
  "ap-northeast-3",
  "ca-central-1",
  "ca-west-1",
  "eu-central-1",
  "eu-central-2",
  "eu-west-1",
  "eu-west-2",
  "eu-west-3",
  "eu-south-1",
  "eu-south-2",
  "eu-north-1",
  "il-central-1",
  "mx-central-1",
  "me-south-1",
  "me-central-1",
  "sa-east-1",
] as const;

/** Value of the "no per-connection region" option — the plugin's precedence
 *  chain then falls through to the endpoint hostname and, finally, to the
 *  default-region plugin setting. */
export const REGION_UNSET = "";

export const REGION_UNSET_LABEL = "Default (plugin setting)";

export interface RegionOption {
  value: string;
  label: string;
}

/** Select options: "use the plugin default" first, then every region. */
export function regionOptions(): RegionOption[] {
  return [
    { value: REGION_UNSET, label: REGION_UNSET_LABEL },
    ...AWS_REGIONS.map((region) => ({ value: region, label: region })),
  ];
}