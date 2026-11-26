import { createRouter, createWebHistory } from "vue-router";

import AssetsDataView from "./views/AssetsDataView.vue";
import ContractsView from "./views/ContractsView.vue";
import DocsView from "./views/DocsView.vue";
import FrontsView from "./views/FrontsView.vue";
import LogsView from "./views/LogsView.vue";

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: "/", redirect: "/fronts" },
    { path: "/fronts", name: "fronts", component: FrontsView },
    { path: "/assets-data", name: "assets-data", component: AssetsDataView },
    { path: "/docs", name: "docs", component: DocsView },
    { path: "/contracts", name: "contracts", component: ContractsView },
    { path: "/logs", name: "logs", component: LogsView },
    { path: "/:pathMatch(.*)*", redirect: "/fronts" },
  ],
});
