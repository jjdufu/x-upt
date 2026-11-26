<script setup lang="ts">
import { onMounted, onUnmounted, reactive, ref, watch } from "vue";

import { ApiRequestError } from "../api/http";
import { fetchJobs, fetchRecentLogs } from "../api/logs";
import type { AccessLogEntry, JobEntry } from "../api/types";
import UiBadge from "../components/ui/UiBadge.vue";
import UiButton from "../components/ui/UiButton.vue";
import UiCard from "../components/ui/UiCard.vue";
import UiDialog from "../components/ui/UiDialog.vue";
import UiInput from "../components/ui/UiInput.vue";
import UiSelect from "../components/ui/UiSelect.vue";
import UiSwitch from "../components/ui/UiSwitch.vue";
import UiTabs from "../components/ui/UiTabs.vue";
import { useToast } from "../composables/useToast";

const { push } = useToast();

const frontFilterOptions = [
  { value: "", label: "全部接入面" },
  { value: "oauth_gateway", label: "oauth_gateway" },
  { value: "sign_gateway", label: "sign_gateway" },
  { value: "rest", label: "rest" },
  { value: "jsonrpc", label: "jsonrpc" },
  { value: "graphql", label: "graphql" },
  { value: "grpc", label: "grpc" },
  { value: "soap", label: "soap" },
  { value: "ws", label: "ws" },
  { value: "job", label: "job" },
  { value: "envelope_token_plain", label: "envelope_token_plain" },
  { value: "envelope_token_crypt", label: "envelope_token_crypt" },
  { value: "envelope_sign_plain", label: "envelope_sign_plain" },
  { value: "envelope_sign_crypt", label: "envelope_sign_crypt" },
  { value: "envelope_cred_plain", label: "envelope_cred_plain" },
  { value: "envelope_cred_crypt", label: "envelope_cred_crypt" },
] as const;

const subTabs = [
  { value: "logs", label: "请求流水", testId: "tab-access-logs" },
  { value: "jobs", label: "异步任务", testId: "tab-jobs" },
] as const;

const activeSubTab = ref("logs");
const loading = ref(false);
const autoRefresh = ref(false);
const filters = reactive({
  front: "",
  status: "",
  request_id: "",
});

const logs = ref<AccessLogEntry[]>([]);
const jobs = ref<JobEntry[]>([]);
const selectedLog = ref<AccessLogEntry | null>(null);
const detailOpen = ref(false);

let pollTimer: number | null = null;

function stopPolling(): void {
  if (pollTimer !== null) {
    window.clearInterval(pollTimer);
    pollTimer = null;
  }
}

function startPolling(): void {
  stopPolling();
  pollTimer = window.setInterval(() => {
    void refreshCurrent();
  }, 3000);
}

watch(autoRefresh, (enabled) => {
  if (enabled) {
    startPolling();
  } else {
    stopPolling();
  }
});

async function refreshCurrent(): Promise<void> {
  loading.value = true;
  try {
    if (activeSubTab.value === "jobs") {
      const res = await fetchJobs();
      jobs.value = res.items;
    } else {
      const res = await fetchRecentLogs({
        front: filters.front,
        status: filters.status,
        request_id: filters.request_id,
        limit: 100,
      });
      logs.value = res.items;
    }
  } catch (err) {
    push(err instanceof ApiRequestError ? err.message : "读取失败", "error");
  } finally {
    loading.value = false;
  }
}

watch(activeSubTab, () => {
  void refreshCurrent();
});

function openDetail(entry: AccessLogEntry): void {
  selectedLog.value = entry;
  detailOpen.value = true;
}

function formatPayload(payload: unknown): string {
  if (payload === undefined || payload === null) {
    return "";
  }
  if (typeof payload === "string") {
    return payload;
  }
  return JSON.stringify(payload, null, 2);
}

onMounted(() => {
  void refreshCurrent();
});

onUnmounted(() => {
  stopPolling();
});
</script>

<template>
  <div class="space-y-4">
    <div class="flex flex-wrap items-center justify-between gap-3">
      <UiTabs
        v-model="activeSubTab"
        :items="subTabs"
      />

      <div class="flex items-center gap-3">
        <label class="flex items-center gap-2 text-xs font-medium text-slate-700">
          <UiSwitch
            v-model="autoRefresh"
            test-id="logs-auto-refresh"
            aria-label="自动刷新"
          />
          <span>自动刷新</span>
        </label>
        <UiButton
          size="sm"
          :loading="loading"
          test-id="logs-refresh-btn"
          @click="refreshCurrent"
        >
          刷新
        </UiButton>
      </div>
    </div>

    <UiCard v-if="activeSubTab === 'logs'" title="最近请求流水">
      <div class="mb-4 grid gap-3 sm:grid-cols-3">
        <UiSelect
          v-model="filters.front"
          :options="frontFilterOptions"
          label="接入面"
          test-id="logs-front-select"
          @update:model-value="refreshCurrent"
        />
        <UiInput
          v-model="filters.status"
          label="状态码"
          placeholder="200"
          test-id="logs-status-input"
          @keyup.enter="refreshCurrent"
        />
        <UiInput
          v-model="filters.request_id"
          label="请求 ID"
          placeholder="req-001"
          test-id="logs-reqid-input"
          @keyup.enter="refreshCurrent"
        />
      </div>

      <div v-if="logs.length === 0" class="py-10 text-center text-sm text-slate-500">
        暂无调用流水
      </div>

      <div v-else class="overflow-x-auto">
        <table class="min-w-full divide-y divide-slate-200 text-left text-xs">
          <thead class="bg-slate-50 text-slate-600">
            <tr>
              <th class="px-2.5 py-2 font-medium">时间</th>
              <th class="px-2.5 py-2 font-medium">请求 ID</th>
              <th class="px-2.5 py-2 font-medium">接入面</th>
              <th class="px-2.5 py-2 font-medium">路径 / RPC</th>
              <th class="px-2.5 py-2 font-medium">鉴权</th>
              <th class="px-2.5 py-2 font-medium">状态</th>
              <th class="px-2.5 py-2 font-medium">耗时</th>
              <th class="px-2.5 py-2 text-right font-medium">详情</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-slate-100">
            <tr v-for="(item, idx) in logs" :key="`${item.request_id}-${idx}`">
              <td class="whitespace-nowrap px-2.5 py-2 font-mono text-slate-500">
                {{ item.ts }}
              </td>
              <td class="max-w-[140px] truncate px-2.5 py-2 font-mono text-slate-800" :title="item.request_id">
                {{ item.request_id }}
              </td>
              <td class="whitespace-nowrap px-2.5 py-2 font-mono text-slate-700">
                {{ item.front }}:{{ item.port }}
              </td>
              <td class="max-w-xs truncate px-2.5 py-2 font-mono text-slate-700">
                {{ item.method }} {{ item.rpc || item.path }}
              </td>
              <td class="px-2.5 py-2 font-mono text-slate-500">
                {{ item.auth || "-" }}
              </td>
              <td class="px-2.5 py-2">
                <UiBadge :tone="item.status < 400 ? 'success' : 'danger'">
                  {{ item.status }}
                </UiBadge>
              </td>
              <td class="whitespace-nowrap px-2.5 py-2 font-mono text-slate-600">
                {{ item.duration }} ms
              </td>
              <td class="px-2.5 py-2 text-right">
                <UiButton size="sm" variant="ghost" @click="openDetail(item)">
                  详情
                </UiButton>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </UiCard>

    <UiCard v-else title="异步任务队列">
      <div v-if="jobs.length === 0" class="py-10 text-center text-sm text-slate-500">
        暂无异步任务
      </div>

      <div v-else class="overflow-x-auto">
        <table class="min-w-full divide-y divide-slate-200 text-left text-xs">
          <thead class="bg-slate-50 text-slate-600">
            <tr>
              <th class="px-3 py-2 font-medium">任务 ID</th>
              <th class="px-3 py-2 font-medium">客户端 ID</th>
              <th class="px-3 py-2 font-medium">业务方法</th>
              <th class="px-3 py-2 font-medium">状态</th>
              <th class="px-3 py-2 font-medium">错误</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-slate-100">
            <tr v-for="j in jobs" :key="j.job_id">
              <td class="px-3 py-2 font-mono text-slate-900">{{ j.job_id }}</td>
              <td class="px-3 py-2 font-mono text-slate-700">{{ j.client_id }}</td>
              <td class="px-3 py-2 font-mono text-slate-700">{{ j.method }}</td>
              <td class="px-3 py-2">
                <UiBadge
                  :tone="
                    j.status === 'succeeded'
                      ? 'success'
                      : j.status === 'failed'
                        ? 'danger'
                        : 'info'
                  "
                >
                  {{ j.status }}
                </UiBadge>
              </td>
              <td class="px-3 py-2 text-rose-600">{{ j.error || "-" }}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </UiCard>

    <UiDialog
      v-model:open="detailOpen"
      :title="selectedLog ? `流水 ${selectedLog.request_id}` : '流水详情'"
      max-width="xl"
    >
      <div v-if="selectedLog" class="space-y-3 text-xs">
        <div class="flex flex-wrap gap-2">
          <UiBadge tone="info">{{ selectedLog.front }}:{{ selectedLog.port }}</UiBadge>
          <UiBadge :tone="selectedLog.status < 400 ? 'success' : 'danger'">
            HTTP {{ selectedLog.status }}
          </UiBadge>
          <UiBadge tone="neutral">{{ selectedLog.duration }} ms</UiBadge>
          <UiBadge v-if="selectedLog.error" tone="danger">{{ selectedLog.error }}</UiBadge>
        </div>
        <div class="space-y-1">
          <span class="font-medium text-slate-700">请求摘要 (in)</span>
          <pre class="max-h-48 overflow-auto rounded-lg bg-slate-900 p-3 font-mono text-slate-100">{{ formatPayload(selectedLog.in) }}</pre>
        </div>
        <div class="space-y-1">
          <span class="font-medium text-slate-700">响应摘要 (out)</span>
          <pre class="max-h-48 overflow-auto rounded-lg bg-slate-900 p-3 font-mono text-slate-100">{{ formatPayload(selectedLog.out) }}</pre>
        </div>
      </div>
    </UiDialog>
  </div>
</template>
