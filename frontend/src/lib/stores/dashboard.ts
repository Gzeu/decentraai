import { writable, derived } from 'svelte/store';
import type { DashboardData, ChartDataPoint } from '$lib/types';

// Main dashboard data store
export const dashboardData = writable<DashboardData | null>(null);

// Loading state
export const isLoading = writable(true);

// Error state
export const error = writable<string | null>(null);

// Auto-refresh interval (5 seconds)
export const refreshInterval = writable(5000);

// WebSocket connection state
export const isConnected = writable(false);

// Derived stores for convenience
export const systemMetrics = derived(
  dashboardData,
  ($data) => $data?.system || null
);

export const networkMetrics = derived(
  dashboardData,
  ($data) => $data?.network || null
);

export const modelsMetrics = derived(
  dashboardData,
  ($data) => $data?.models || null
);

export const workersMetrics = derived(
  dashboardData,
  ($data) => $data?.workers || null
);

export const performanceMetrics = derived(
  dashboardData,
  ($data) => $data?.performance || null
);

// Chart data history (last 60 points = 5 minutes with 5s refresh)
export const cpuHistory = writable<ChartDataPoint[]>([]);
export const memoryHistory = writable<ChartDataPoint[]>([]);
export const latencyHistory = writable<ChartDataPoint[]>([]);
export const throughputHistory = writable<ChartDataPoint[]>([]);

// Functions to update chart history
export function updateHistory(
  store: typeof cpuHistory,
  value: number,
  timestamp: string
) {
  store.update(history => {
    const newPoint = { timestamp, value };
    const updated = [...history, newPoint];
    // Keep only last 60 points
    return updated.slice(-60);
  });
}

// Reset all stores
export function resetDashboard() {
  dashboardData.set(null);
  isLoading.set(true);
  error.set(null);
  isConnected.set(false);
  cpuHistory.set([]);
  memoryHistory.set([]);
  latencyHistory.set([]);
  throughputHistory.set([]);
}