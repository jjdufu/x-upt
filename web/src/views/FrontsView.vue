<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { useRouter } from "vue-router";

import {
  disableFront,
  disableMethod,
  enableFront,
  enableMethod,
  updateFront,
} from "../api/fronts";
import { ApiRequestError } from "../api/http";
import { fetchSystemStatus, updateSettings } from "../api/status";
import type {
  BusinessMethodId,
  FrontStatus,
  FrontUpdatePayload,
  GlobalSettings,
  MethodStatus,
  SystemStatus,
} from "../api/types";
import MethodSwitchCard from "../components/MethodSwitchCard.vue";
import UiBadge from "../components/ui/UiBadge.vue";
import UiButton from "../components/ui/UiButton.vue";
import UiCard from "../components/ui/UiCard.vue";
import UiDialog from "../components/ui/UiDialog.vue";
import UiInput from "../components/ui/UiInput.vue";
import UiSwitch from "../components/ui/UiSwitch.vue";
import { useToast } from "../composables/useToast";

const router = useRouter();
const { push } = useToast();

const loading = ref(true);
const statusData = ref<SystemStatus | null>(null);
const busyKeys = reactive<Record<string, boolean>>({});

const settingsOpen = ref(false);
const settingsSaving = ref(false);
const settingsForm = reactive<GlobalSettings>({
  latency_min_ms: 20,
  latency_max_ms: 100,
  token_expire_minutes: 30,
  token_cleanup_seconds: 300,
  token_max_per_client: 0,
  sign_window_seconds: 300,
  sign_nonce_ttl_seconds: 600,
  job_expire_seconds: 600,
});

const frontDialogOpen = ref(false);
const frontSaving = ref(false);
const editingFront = ref<FrontStatus | null>(null);
const frontForm = reactive({
  host: "0.0.0.0",
  port: 16000,
  http2: false,
  h2c: false,
  expire_minutes: 30,
  auth: [] as string[],
  tls_enabled: false,
  tls_cert_pem: "",
  tls_key_pem: "",
  tls_client_ca_pem: "",
  ws_max_conn: 1024,
  ws_ping_secs: 30,
  job_workers: 4,
});

const coreFronts = computed(() =>
  (statusData.value?.fronts ?? []).filter((f) => f.category === "core"),
);
const envelopeFronts = computed(() =>
  (statusData.value?.fronts ?? []).filter((f) => f.category === "envelope"),
);

async function refreshAll(): Promise<void> {
  loading.value = true;
  try {
    const res = await fetchSystemStatus();
    statusData.value = res;
    Object.assign(settingsForm, res.settings);
  } catch (err) {
    push(err instanceof ApiRequestError ? err.message : "状态读取失败", "error");
  } finally {
    loading.value = false;
  }
}

async function onToggleMethod(item: MethodStatus, next: boolean): Promise<void> {
  const key = `method:${item.method}`;
  busyKeys[key] = true;
  try {
    if (next) {
      await enableMethod(item.method);
      push("已启用", "success");
    } else {
      await disableMethod(item.method);
      push("已停用", "info");
    }
    await refreshAll();
  } catch (err) {
    push(err instanceof ApiRequestError ? err.message : "操作失败", "error");
    await refreshAll();
  } finally {
    busyKeys[key] = false;
  }
}

function onGotoImport(item: MethodStatus): void {
  const tab = methodTargetTab(item.method);
  void router.push({ path: "/assets-data", query: { tab } });
}

function methodTargetTab(method: BusinessMethodId): string {
  if (method === "x_upt.posts.search") {
    return "posts";
  }
  return "images";
}

async function onToggleFront(front: FrontStatus, next: boolean): Promise<void> {
  const key = `front:${front.name}`;
  busyKeys[key] = true;
  try {
    if (next) {
      await enableFront(front.name);
      push("已启用", "success");
    } else {
      await disableFront(front.name);
      push("已停用", "info");
    }
    await refreshAll();
  } catch (err) {
    push(err instanceof ApiRequestError ? err.message : "操作失败", "error");
    await refreshAll();
  } finally {
    busyKeys[key] = false;
  }
}

function frontStatusText(front: FrontStatus): string {
  if (front.enabled) {
    return "运行中";
  }
  if (!front.can_enable) {
    return "未就绪";
  }
  return "已停用";
}

function frontStatusTone(front: FrontStatus): "success" | "warning" | "neutral" {
  if (front.enabled) {
    return "success";
  }
  if (!front.can_enable) {
    return "warning";
  }
  return "neutral";
}

function openFrontConfig(front: FrontStatus): void {
  editingFront.value = front;
  frontForm.host = front.host;
  frontForm.port = front.port;
  frontForm.http2 = front.http2;
  frontForm.h2c = front.h2c;
  frontForm.expire_minutes = front.expire_minutes;
  frontForm.auth = [...front.auth];
  frontForm.tls_enabled = front.tls_enabled;
  frontForm.tls_cert_pem = "";
  frontForm.tls_key_pem = "";
  frontForm.tls_client_ca_pem = "";
  frontForm.ws_max_conn = front.ws_max_conn;
  frontForm.ws_ping_secs = front.ws_ping_secs;
  frontForm.job_workers = front.job_workers;
  frontDialogOpen.value = true;
}

function toggleAuthMode(mode: string): void {
  if (frontForm.auth.includes(mode)) {
    frontForm.auth = frontForm.auth.filter((a) => a !== mode);
  } else {
    frontForm.auth = [...frontForm.auth, mode];
  }
}

async function saveFrontConfig(): Promise<void> {
  const target = editingFront.value;
  if (!target) {
    return;
  }
  frontSaving.value = true;
  try {
    const payload: FrontUpdatePayload = {
      host: frontForm.host.trim() || "0.0.0.0",
      port: Number(frontForm.port),
      http2: frontForm.http2,
      h2c: frontForm.h2c,
      expire_minutes: Number(frontForm.expire_minutes),
      auth: [...frontForm.auth],
      tls_enabled: frontForm.tls_enabled,
      ws_max_conn: Number(frontForm.ws_max_conn),
      ws_ping_secs: Number(frontForm.ws_ping_secs),
      job_workers: Number(frontForm.job_workers),
    };
    if (frontForm.tls_cert_pem.trim().length > 0) {
      payload.tls_cert_pem = frontForm.tls_cert_pem.trim();
    }
    if (frontForm.tls_key_pem.trim().length > 0) {
      payload.tls_key_pem = frontForm.tls_key_pem.trim();
    }
    if (frontForm.tls_client_ca_pem.trim().length > 0) {
      payload.tls_client_ca_pem = frontForm.tls_client_ca_pem.trim();
    }
    await updateFront(target.name, payload);
    frontDialogOpen.value = false;
    push("已保存", "success");
    await refreshAll();
  } catch (err) {
    push(err instanceof ApiRequestError ? err.message : "保存失败", "error");
  } finally {
    frontSaving.value = false;
  }
}

async function saveGlobalSettings(): Promise<void> {
  settingsSaving.value = true;
  try {
    const updated = await updateSettings({
      latency_min_ms: Number(settingsForm.latency_min_ms),
      latency_max_ms: Number(settingsForm.latency_max_ms),
      token_expire_minutes: Number(settingsForm.token_expire_minutes),
      token_cleanup_seconds: Number(settingsForm.token_cleanup_seconds),
      token_max_per_client: Number(settingsForm.token_max_per_client),
      sign_window_seconds: Number(settingsForm.sign_window_seconds),
      sign_nonce_ttl_seconds: Number(settingsForm.sign_nonce_ttl_seconds),
      job_expire_seconds: Number(settingsForm.job_expire_seconds),
    });
    Object.assign(settingsForm, updated);
    settingsOpen.value = false;
    push("已保存", "success");
    await refreshAll();
  } catch (err) {
    push(err instanceof ApiRequestError ? err.message : "保存失败", "error");
  } finally {
    settingsSaving.value = false;
  }
}

onMounted(() => {
  void refreshAll();
});
</script>

<template>
  <div class="space-y-6">
    <section class="flex flex-wrap items-center justify-between gap-3">
      <div class="flex flex-wrap items-center gap-2 text-xs text-slate-600">
        <UiBadge v-if="statusData" tone="info" test-id="admin-listen-badge">
          控制台 {{ statusData.admin_listen }}
        </UiBadge>
        <UiBadge v-if="statusData" tone="neutral">
          v{{ statusData.version }}
        </UiBadge>
        <UiBadge v-if="statusData" tone="neutral" test-id="summary-images-count">
          图片 {{ statusData.counts.images }}
        </UiBadge>
        <UiBadge v-if="statusData" tone="neutral" test-id="summary-posts-count">
          帖子 {{ statusData.counts.posts }}
        </UiBadge>
        <UiBadge v-if="statusData" tone="neutral" test-id="summary-apps-count">
          凭据 {{ statusData.counts.apps }}
        </UiBadge>
        <UiBadge v-if="statusData" tone="neutral" test-id="summary-secrets-count">
          密钥 {{ statusData.counts.envelope_secrets }}
        </UiBadge>
      </div>

      <div class="flex items-center gap-2">
        <UiButton
          size="sm"
          test-id="open-settings"
          @click="settingsOpen = true"
        >
          全局参数
        </UiButton>
        <UiButton
          size="sm"
          :loading="loading"
          test-id="refresh-status"
          @click="refreshAll"
        >
          刷新
        </UiButton>
      </div>
    </section>

    <section class="space-y-3">
      <div class="flex items-center justify-between">
        <h2 class="text-sm font-semibold text-slate-900">业务子接口</h2>
      </div>
      <div class="grid gap-4 md:grid-cols-3">
        <MethodSwitchCard
          v-for="m in statusData?.methods ?? []"
          :key="m.method"
          :item="m"
          :busy="Boolean(busyKeys[`method:${m.method}`])"
          @toggle="onToggleMethod"
          @goto-import="onGotoImport"
        />
      </div>
    </section>

    <section class="space-y-3">
      <h2 class="text-sm font-semibold text-slate-900">核心协议接入面</h2>
      <div class="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
        <UiCard
          v-for="f in coreFronts"
          :key="f.name"
          :test-id="`front-card-${f.name}`"
          padding="sm"
        >
          <div class="flex items-start justify-between gap-3">
            <div class="min-w-0">
              <div class="flex flex-wrap items-center gap-2">
                <span class="font-mono text-sm font-semibold text-slate-900">
                  {{ f.name }}
                </span>
                <UiBadge :tone="frontStatusTone(f)" :test-id="`front-badge-${f.name}`">
                  {{ frontStatusText(f) }}
                </UiBadge>
                <UiBadge v-if="f.tls_enabled" tone="info">TLS</UiBadge>
              </div>
              <p class="mt-0.5 text-xs text-slate-500">
                {{ f.protocol }} · <span class="font-mono">{{ f.host }}:{{ f.port }}</span>
              </p>
              <p v-if="f.auth.length > 0" class="mt-1 text-[11px] text-slate-500">
                鉴权: {{ f.auth.join(", ") }}
              </p>
            </div>

            <UiSwitch
              :model-value="f.enabled"
              :disabled="Boolean(busyKeys[`front:${f.name}`]) || (!f.enabled && !f.can_enable)"
              :test-id="`front-switch-${f.name}`"
              :aria-label="f.name"
              @update:model-value="(next) => onToggleFront(f, next)"
            />
          </div>

          <div
            v-if="!f.enabled && f.missing_preconditions.length > 0"
            :data-testid="`front-missing-${f.name}`"
            class="mt-3 rounded-lg border border-amber-200 bg-amber-50/70 px-3 py-2 text-xs text-amber-900"
          >
            <ul class="space-y-0.5">
              <li v-for="(reason, idx) in f.missing_preconditions" :key="idx">
                {{ reason }}
              </li>
            </ul>
          </div>

          <div class="mt-3 flex items-center justify-between border-t border-slate-100 pt-2.5">
            <RouterLink
              to="/assets-data?tab=apps"
              class="text-xs font-medium text-slate-600 hover:text-slate-900"
            >
              数据与素材
            </RouterLink>
            <UiButton
              size="sm"
              variant="ghost"
              :test-id="`front-config-${f.name}`"
              @click="openFrontConfig(f)"
            >
              配置
            </UiButton>
          </div>
        </UiCard>
      </div>
    </section>

    <section class="space-y-3">
      <h2 class="text-sm font-semibold text-slate-900">信封协议接入面</h2>
      <div class="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
        <UiCard
          v-for="f in envelopeFronts"
          :key="f.name"
          :test-id="`front-card-${f.name}`"
          padding="sm"
        >
          <div class="flex items-start justify-between gap-3">
            <div class="min-w-0">
              <div class="flex flex-wrap items-center gap-2">
                <span class="font-mono text-sm font-semibold text-slate-900">
                  {{ f.name }}
                </span>
                <UiBadge :tone="frontStatusTone(f)" :test-id="`front-badge-${f.name}`">
                  {{ frontStatusText(f) }}
                </UiBadge>
              </div>
              <p class="mt-0.5 text-xs text-slate-500">
                {{ f.protocol }} · <span class="font-mono">{{ f.host }}:{{ f.port }}</span>
              </p>
              <p class="mt-1 text-[11px] text-slate-500">
                画像 {{ f.profile_count }}
              </p>
            </div>

            <UiSwitch
              :model-value="f.enabled"
              :disabled="Boolean(busyKeys[`front:${f.name}`]) || (!f.enabled && !f.can_enable)"
              :test-id="`front-switch-${f.name}`"
              :aria-label="f.name"
              @update:model-value="(next) => onToggleFront(f, next)"
            />
          </div>

          <div
            v-if="!f.enabled && f.missing_preconditions.length > 0"
            :data-testid="`front-missing-${f.name}`"
            class="mt-3 rounded-lg border border-amber-200 bg-amber-50/70 px-3 py-2 text-xs text-amber-900"
          >
            <ul class="space-y-0.5">
              <li v-for="(reason, idx) in f.missing_preconditions" :key="idx">
                {{ reason }}
              </li>
            </ul>
          </div>

          <div class="mt-3 flex items-center justify-between border-t border-slate-100 pt-2.5">
            <RouterLink
              :to="`/assets-data?tab=envelopes&front=${encodeURIComponent(f.name)}`"
              class="text-xs font-medium text-slate-600 hover:text-slate-900"
            >
              数据与素材
            </RouterLink>
            <UiButton
              size="sm"
              variant="ghost"
              :test-id="`front-config-${f.name}`"
              @click="openFrontConfig(f)"
            >
              配置
            </UiButton>
          </div>
        </UiCard>
      </div>
    </section>

    <UiDialog
      v-model:open="settingsOpen"
      title="全局参数"
      test-id="settings-dialog"
    >
      <form class="space-y-4" @submit.prevent="saveGlobalSettings">
        <div class="grid gap-3 sm:grid-cols-2">
          <UiInput
            :model-value="settingsForm.latency_min_ms"
            type="number"
            :min="0"
            label="最小延迟 (ms)"
            placeholder="20"
            @update:model-value="(v) => (settingsForm.latency_min_ms = Number(v))"
          />
          <UiInput
            :model-value="settingsForm.latency_max_ms"
            type="number"
            :min="0"
            label="最大延迟 (ms)"
            placeholder="100"
            @update:model-value="(v) => (settingsForm.latency_max_ms = Number(v))"
          />
          <UiInput
            :model-value="settingsForm.token_expire_minutes"
            type="number"
            :min="1"
            label="令牌有效期 (分)"
            placeholder="30"
            @update:model-value="(v) => (settingsForm.token_expire_minutes = Number(v))"
          />
          <UiInput
            :model-value="settingsForm.token_cleanup_seconds"
            type="number"
            :min="1"
            label="清理周期 (秒)"
            placeholder="300"
            @update:model-value="(v) => (settingsForm.token_cleanup_seconds = Number(v))"
          />
          <UiInput
            :model-value="settingsForm.token_max_per_client"
            type="number"
            :min="0"
            label="单客户端令牌上限"
            placeholder="0"
            @update:model-value="(v) => (settingsForm.token_max_per_client = Number(v))"
          />
          <UiInput
            :model-value="settingsForm.sign_window_seconds"
            type="number"
            :min="1"
            label="签名时间窗 (秒)"
            placeholder="300"
            @update:model-value="(v) => (settingsForm.sign_window_seconds = Number(v))"
          />
          <UiInput
            :model-value="settingsForm.sign_nonce_ttl_seconds"
            type="number"
            :min="1"
            label="Nonce 保留 (秒)"
            placeholder="600"
            @update:model-value="(v) => (settingsForm.sign_nonce_ttl_seconds = Number(v))"
          />
          <UiInput
            :model-value="settingsForm.job_expire_seconds"
            type="number"
            :min="1"
            label="异步任务过期 (秒)"
            placeholder="600"
            @update:model-value="(v) => (settingsForm.job_expire_seconds = Number(v))"
          />
        </div>
        <div class="flex justify-end gap-2 pt-2">
          <UiButton @click="settingsOpen = false">取消</UiButton>
          <UiButton
            type="submit"
            variant="primary"
            :loading="settingsSaving"
            test-id="save-settings"
          >
            保存
          </UiButton>
        </div>
      </form>
    </UiDialog>

    <UiDialog
      v-model:open="frontDialogOpen"
      :title="editingFront ? `配置 ${editingFront.name}` : '配置接入面'"
      max-width="xl"
      test-id="front-config-dialog"
    >
      <form v-if="editingFront" class="space-y-4" @submit.prevent="saveFrontConfig">
        <div class="grid gap-3 sm:grid-cols-3">
          <UiInput
            v-model="frontForm.host"
            label="监听地址"
            placeholder="0.0.0.0"
            test-id="front-host-input"
          />
          <UiInput
            :model-value="frontForm.port"
            type="number"
            :min="1"
            :max="65535"
            label="端口"
            placeholder="16002"
            test-id="front-port-input"
            @update:model-value="(v) => (frontForm.port = Number(v))"
          />
          <UiInput
            :model-value="frontForm.expire_minutes"
            type="number"
            :min="1"
            label="令牌有效期 (分)"
            placeholder="30"
            @update:model-value="(v) => (frontForm.expire_minutes = Number(v))"
          />
        </div>

        <div class="flex flex-wrap items-center gap-6 rounded-lg border border-slate-200 p-3">
          <label class="flex items-center gap-2 text-xs font-medium text-slate-700">
            <UiSwitch v-model="frontForm.http2" aria-label="HTTP/2" />
            <span>HTTP/2</span>
          </label>
          <label class="flex items-center gap-2 text-xs font-medium text-slate-700">
            <UiSwitch v-model="frontForm.h2c" aria-label="h2c" />
            <span>h2c</span>
          </label>
          <label class="flex items-center gap-2 text-xs font-medium text-slate-700">
            <UiSwitch v-model="frontForm.tls_enabled" aria-label="TLS" />
            <span>TLS</span>
          </label>
        </div>

        <div v-if="editingFront.supported_auth.length > 0" class="space-y-2">
          <span class="block text-xs font-medium text-slate-700">支持鉴权模式</span>
          <div class="flex flex-wrap gap-2">
            <label
              v-for="mode in editingFront.supported_auth"
              :key="mode"
              class="inline-flex cursor-pointer items-center gap-1.5 rounded-md border px-2.5 py-1 text-xs"
              :class="
                frontForm.auth.includes(mode)
                  ? 'border-slate-900 bg-slate-900 text-white'
                  : 'border-slate-200 bg-white text-slate-700'
              "
            >
              <input
                type="checkbox"
                class="sr-only"
                :checked="frontForm.auth.includes(mode)"
                @change="toggleAuthMode(mode)"
              />
              <span class="font-mono">{{ mode }}</span>
            </label>
          </div>
        </div>

        <div v-if="frontForm.tls_enabled" class="grid gap-3 sm:grid-cols-3">
          <label class="block">
            <span class="mb-1 block text-xs font-medium text-slate-700">
              服务端证书 PEM {{ editingFront.has_tls_cert ? "(已存)" : "" }}
            </span>
            <textarea
              v-model="frontForm.tls_cert_pem"
              rows="3"
              placeholder="-----BEGIN CERTIFICATE-----"
              class="block w-full rounded-lg border border-slate-200 p-2 font-mono text-xs focus:border-slate-900 focus:outline-none"
            />
          </label>
          <label class="block">
            <span class="mb-1 block text-xs font-medium text-slate-700">
              服务端私钥 PEM {{ editingFront.has_tls_key ? "(已存)" : "" }}
            </span>
            <textarea
              v-model="frontForm.tls_key_pem"
              rows="3"
              placeholder="-----BEGIN PRIVATE KEY-----"
              class="block w-full rounded-lg border border-slate-200 p-2 font-mono text-xs focus:border-slate-900 focus:outline-none"
            />
          </label>
          <label class="block">
            <span class="mb-1 block text-xs font-medium text-slate-700">
              客户端 CA PEM {{ editingFront.has_tls_client_ca ? "(已存)" : "" }}
            </span>
            <textarea
              v-model="frontForm.tls_client_ca_pem"
              rows="3"
              placeholder="-----BEGIN CERTIFICATE-----"
              class="block w-full rounded-lg border border-slate-200 p-2 font-mono text-xs focus:border-slate-900 focus:outline-none"
            />
          </label>
        </div>

        <div class="grid gap-3 sm:grid-cols-3">
          <UiInput
            :model-value="frontForm.ws_max_conn"
            type="number"
            :min="1"
            label="WS 最大连接"
            placeholder="1024"
            @update:model-value="(v) => (frontForm.ws_max_conn = Number(v))"
          />
          <UiInput
            :model-value="frontForm.ws_ping_secs"
            type="number"
            :min="1"
            label="WS 心跳 (秒)"
            placeholder="30"
            @update:model-value="(v) => (frontForm.ws_ping_secs = Number(v))"
          />
          <UiInput
            :model-value="frontForm.job_workers"
            type="number"
            :min="1"
            label="任务工作协程"
            placeholder="4"
            @update:model-value="(v) => (frontForm.job_workers = Number(v))"
          />
        </div>

        <div class="flex justify-end gap-2 pt-2">
          <UiButton @click="frontDialogOpen = false">取消</UiButton>
          <UiButton
            type="submit"
            variant="primary"
            :loading="frontSaving"
            test-id="front-config-save"
          >
            保存
          </UiButton>
        </div>
      </form>
    </UiDialog>
  </div>
</template>
