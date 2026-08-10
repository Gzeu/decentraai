<script lang="ts">
  let { status }: { status: 'active' | 'busy' | 'offline' } = $props();

  const statusConfig = {
    active: {
      label: 'Active',
      class: 'bg-green-500/20 text-green-400 border-green-500/30'
    },
    busy: {
      label: 'Busy',
      class: 'bg-yellow-500/20 text-yellow-400 border-yellow-500/30'
    },
    offline: {
      label: 'Offline',
      class: 'bg-red-500/20 text-red-400 border-red-500/30'
    }
  };

  const config = $derived(statusConfig[status]);
</script>

<span class="inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium border {config.class}">
  <span class="w-1.5 h-1.5 rounded-full bg-current mr-1.5"></span>
  {config.label}
</span>