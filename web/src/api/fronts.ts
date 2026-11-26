import { isArrayOf, isRecord, requestJson } from "./http";
import { isFrontStatus, isMethodStatus } from "./status";
import type {
  BusinessMethodId,
  FrontStatus,
  FrontUpdatePayload,
  MethodStatus,
} from "./types";

function isFrontListResponse(value: unknown): value is { items: FrontStatus[] } {
  return isRecord(value) && isArrayOf(value.items, isFrontStatus);
}

export function enableMethod(method: BusinessMethodId): Promise<MethodStatus> {
  return requestJson(
    `/api/v1/methods/${encodeURIComponent(method)}/enable`,
    isMethodStatus,
    { method: "POST" },
  );
}

export function disableMethod(method: BusinessMethodId): Promise<MethodStatus> {
  return requestJson(
    `/api/v1/methods/${encodeURIComponent(method)}/disable`,
    isMethodStatus,
    { method: "POST" },
  );
}

export function fetchFronts(): Promise<{ items: FrontStatus[] }> {
  return requestJson("/api/v1/fronts", isFrontListResponse);
}

export function updateFront(
  front: string,
  payload: FrontUpdatePayload,
): Promise<FrontStatus> {
  return requestJson(
    `/api/v1/fronts/${encodeURIComponent(front)}`,
    isFrontStatus,
    { method: "PUT", body: payload },
  );
}

export function enableFront(front: string): Promise<FrontStatus> {
  return requestJson(
    `/api/v1/fronts/${encodeURIComponent(front)}/enable`,
    isFrontStatus,
    { method: "POST" },
  );
}

export function disableFront(front: string): Promise<FrontStatus> {
  return requestJson(
    `/api/v1/fronts/${encodeURIComponent(front)}/disable`,
    isFrontStatus,
    { method: "POST" },
  );
}
