import {
  isArrayOf,
  isBoolean,
  isNumber,
  isOptionalNullableString,
  isRecord,
  isString,
  requestJson,
} from "./http";
import type {
  EnvelopeProfilesResponse,
  SecretMeta,
  SecretUpsertPayload,
} from "./types";

export function isSecretMeta(value: unknown): value is SecretMeta {
  return (
    isRecord(value) &&
    isString(value.name) &&
    isString(value.secret_kind) &&
    isNumber(value.byte_len) &&
    isString(value.fingerprint) &&
    isOptionalNullableString(value.public_material) &&
    isString(value.updated_at)
  );
}

function isSecretListOrSingle(
  value: unknown,
): value is { items: SecretMeta[] } | SecretMeta {
  return (
    isSecretMeta(value) ||
    (isRecord(value) && isArrayOf(value.items, isSecretMeta))
  );
}

function isDeletedResponse(value: unknown): value is { deleted: boolean } {
  return isRecord(value) && isBoolean(value.deleted);
}

export function isEnvelopeProfilesResponse(
  value: unknown,
): value is EnvelopeProfilesResponse {
  return (
    isRecord(value) &&
    isString(value.front) &&
    Array.isArray(value.profiles)
  );
}

export async function fetchEnvelopeSecrets(): Promise<{ items: SecretMeta[] }> {
  const res = await requestJson("/api/v1/envelopes/secrets", isSecretListOrSingle);
  if ("items" in res) {
    return res;
  }
  return { items: [res] };
}

export async function upsertEnvelopeSecret(
  payload: SecretUpsertPayload,
): Promise<{ items: SecretMeta[] }> {
  const res = await requestJson("/api/v1/envelopes/secrets", isSecretListOrSingle, {
    method: "POST",
    body: payload,
  });
  if ("items" in res) {
    return res;
  }
  return { items: [res] };
}

export function deleteEnvelopeSecret(name: string): Promise<{ deleted: boolean }> {
  return requestJson(
    `/api/v1/envelopes/secrets/${encodeURIComponent(name)}`,
    isDeletedResponse,
    { method: "DELETE" },
  );
}

export function fetchEnvelopeProfiles(
  front: string,
): Promise<EnvelopeProfilesResponse> {
  return requestJson(
    `/api/v1/envelopes/${encodeURIComponent(front)}/profiles`,
    isEnvelopeProfilesResponse,
  );
}

export function replaceEnvelopeProfiles(
  front: string,
  profiles: unknown[],
): Promise<EnvelopeProfilesResponse> {
  return requestJson(
    `/api/v1/envelopes/${encodeURIComponent(front)}/profiles`,
    isEnvelopeProfilesResponse,
    {
      method: "PUT",
      body: { profiles },
    },
  );
}
