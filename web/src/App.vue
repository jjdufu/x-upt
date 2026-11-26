<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { RouterLink, RouterView, useRoute } from "vue-router";

import { ApiRequestError } from "./api/http";
import { fetchAuthSession, loginAdmin, logoutAdmin } from "./api/status";
import UiButton from "./components/ui/UiButton.vue";
import UiCard from "./components/ui/UiCard.vue";
import UiInput from "./components/ui/UiInput.vue";
import { useToast } from "./composables/useToast";

const route = useRoute();
const { toasts, push, dismiss } = useToast();

const sessionLoading = ref(true);
const authRequired = ref(false);
const authenticated = ref(true);
const tokenInput = ref("");
const loginSubmitting = ref(false);
const loginError = ref("");

const navItems = [
  { to: "/fronts", label: "接入面", testId: "nav-fronts" },
  { to: "/assets-data", label: "数据与素材", testId: "nav-assets" },
  { to: "/docs", label: "接口文档", testId: "nav-docs" },
  { to: "/contracts", label: "协议契约", testId: "nav-contracts" },
  { to: "/logs", label: "调用日志", testId: "nav-logs" },
] as const;

const showLoginGate = computed(
  () => !sessionLoading.value && authRequired.value && !authenticated.value,
);

async function loadSession(): Promise<void> {
  sessionLoading.value = true;
  try {
    const sess = await fetchAuthSession();
    authRequired.value = sess.auth_required;
    authenticated.value = sess.authenticated;
  } catch {
    authRequired.value = false;
    authenticated.value = true;
  } finally {
    sessionLoading.value = false;
  }
}

async function handleLogin(): Promise<void> {
  const trimmed = tokenInput.value.trim();
  if (!trimmed) {
    loginError.value = "口令为空";
    return;
  }
  loginSubmitting.value = true;
  loginError.value = "";
  try {
    const res = await loginAdmin(trimmed);
    authenticated.value = res.authenticated;
    tokenInput.value = "";
    if (!res.authenticated) {
      loginError.value = "认证失败";
    }
  } catch (err) {
    loginError.value =
      err instanceof ApiRequestError ? err.message : "认证失败";
  } finally {
    loginSubmitting.value = false;
  }
}

async function handleLogout(): Promise<void> {
  try {
    const res = await logoutAdmin();
    authenticated.value = res.authenticated;
    push("已退出", "info");
  } catch (err) {
    push(err instanceof ApiRequestError ? err.message : "退出失败", "error");
  }
}

onMounted(() => {
  void loadSession();
});
</script>

<template>
  <div class="min-h-screen bg-slate-50 text-slate-900">
    <header class="sticky top-0 z-30 border-b border-slate-200 bg-white/95 backdrop-blur">
      <div class="mx-auto flex max-w-7xl flex-wrap items-center justify-between gap-4 px-4 py-3 sm:px-6">
        <div class="flex items-center gap-3">
          <div class="flex h-8 w-8 items-center justify-center rounded-lg bg-slate-900 text-xs font-bold tracking-tight text-white">
            XU
          </div>
          <div class="flex items-baseline gap-2">
            <span class="text-base font-semibold tracking-tight text-slate-900">x-upt</span>
            <span class="text-xs text-slate-500">模拟上游控制台</span>
          </div>
        </div>

        <nav
          v-if="!showLoginGate && !sessionLoading"
          class="flex flex-wrap items-center gap-1"
          aria-label="主导航"
        >
          <RouterLink
            v-for="item in navItems"
            :key="item.to"
            :to="item.to"
            :data-testid="item.testId"
            class="rounded-lg px-3 py-1.5 text-sm font-medium transition-colors"
            :class="
              route.path === item.to
                ? 'bg-slate-900 text-white'
                : 'text-slate-600 hover:bg-slate-100 hover:text-slate-900'
            "
          >
            {{ item.label }}
          </RouterLink>
        </nav>

        <div v-if="authRequired && authenticated && !sessionLoading" class="flex items-center gap-2">
          <UiButton
            size="sm"
            variant="ghost"
            test-id="logout-btn"
            @click="handleLogout"
          >
            退出
          </UiButton>
        </div>
      </div>
    </header>

    <main class="mx-auto max-w-7xl px-4 py-6 sm:px-6">
      <div
        v-if="sessionLoading"
        class="flex items-center justify-center py-20 text-sm text-slate-500"
      >
        加载中…
      </div>

      <div
        v-else-if="showLoginGate"
        class="mx-auto max-w-sm py-16"
      >
        <UiCard title="管理口令" test-id="login-card">
          <form class="space-y-4" @submit.prevent="handleLogin">
            <UiInput
              v-model="tokenInput"
              type="password"
              label="管理口令"
              placeholder="admin-token"
              test-id="login-token-input"
            />
            <p v-if="loginError" class="text-xs text-rose-600">
              {{ loginError }}
            </p>
            <div class="flex justify-end">
              <UiButton
                type="submit"
                variant="primary"
                :loading="loginSubmitting"
                test-id="login-submit"
              >
                登录
              </UiButton>
            </div>
          </form>
        </UiCard>
      </div>

      <RouterView v-else />
    </main>

    <div
      class="pointer-events-none fixed right-4 bottom-4 z-50 flex max-w-sm flex-col gap-2"
      aria-live="polite"
    >
      <div
        v-for="t in toasts"
        :key="t.id"
        class="pointer-events-auto flex items-center justify-between gap-3 rounded-lg border px-3.5 py-2.5 text-xs font-medium shadow-md"
        :class="
          t.tone === 'error'
            ? 'border-rose-200 bg-rose-50 text-rose-800'
            : t.tone === 'success'
              ? 'border-emerald-200 bg-emerald-50 text-emerald-800'
              : 'border-slate-200 bg-white text-slate-800'
        "
      >
        <span>{{ t.text }}</span>
        <button
          type="button"
          class="text-current opacity-60 hover:opacity-100"
          aria-label="关闭"
          @click="dismiss(t.id)"
        >
          ×
        </button>
      </div>
    </div>
  </div>
</template>
