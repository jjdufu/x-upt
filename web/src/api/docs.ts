import {
  isArrayOf,
  isBoolean,
  isNumber,
  isRecord,
  isString,
  requestBytes,
  requestJson,
  requestText,
} from "./http";
import { isFrontCategory } from "./status";
import type { ContractName, EndpointDoc, FrontDoc } from "./types";

export const ALL_CONTRACTS: ReadonlyArray<{
  name: ContractName;
  filename: string;
  binary: boolean;
}> = [
  { name: "grpc.proto", filename: "x_upt.proto", binary: false },
  { name: "grpc.fds", filename: "x_upt.fds", binary: true },
  { name: "soap.wsdl", filename: "x_upt.wsdl", binary: false },
  { name: "graphql.sdl", filename: "schema.graphql", binary: false },
  {
    name: "upstream-openapi.json",
    filename: "upstream-openapi.json",
    binary: false,
  },
  {
    name: "admin-openapi.json",
    filename: "admin-openapi.json",
    binary: false,
  },
];

export function isEndpointDoc(value: unknown): value is EndpointDoc {
  return (
    isRecord(value) &&
    isString(value.method) &&
    isString(value.path) &&
    isString(value.description) &&
    isString(value.request_example) &&
    isString(value.response_example) &&
    isString(value.curl_example)
  );
}

export function isFrontDoc(value: unknown): value is FrontDoc {
  return (
    isRecord(value) &&
    isString(value.name) &&
    isFrontCategory(value.category) &&
    isString(value.protocol) &&
    isString(value.host) &&
    isNumber(value.port) &&
    isBoolean(value.enabled) &&
    isString(value.summary) &&
    isString(value.auth_notes) &&
    isArrayOf(value.endpoints, isEndpointDoc)
  );
}

function isFrontDocsResponse(value: unknown): value is { items: FrontDoc[] } {
  return isRecord(value) && isArrayOf(value.items, isFrontDoc);
}

export function fetchFrontDocs(): Promise<{ items: FrontDoc[] }> {
  return requestJson("/api/v1/docs/fronts", isFrontDocsResponse);
}

export function contractDownloadUrl(name: ContractName): string {
  return `/api/v1/contracts/${encodeURIComponent(name)}?download=1`;
}

export function fetchContractText(name: ContractName): Promise<string> {
  return requestText(`/api/v1/contracts/${encodeURIComponent(name)}`);
}

export function fetchContractBytes(name: ContractName): Promise<Uint8Array> {
  return requestBytes(`/api/v1/contracts/${encodeURIComponent(name)}`);
}
