<script lang="ts">
  let { currentPage }: { currentPage: 'monitoring' | 'chat' } = $props();
</script>

<nav class="navigation">
  <div class="nav-header">
    <h1>🤖 DecentraAI</h1>
  </div>
  <div class="nav-links">
    <a 
      href="/" 
      class="nav-link {currentPage === 'monitoring' ? 'active' : ''}"
    >
      📊 Monitoring
    </a>
    <a 
      href="/chat" 
      class="nav-link {currentPage === 'chat' ? 'active' : ''}"
    >
      💬 Chat
    </a>
  </div>
</nav>

<style>
  .navigation {
    background: rgba(30, 41, 59, 0.95);
    border-bottom: 1px solid rgba(75, 85, 99, 0.5);
    padding: 1rem 2rem;
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .nav-header h1 {
    font-size: 1.25rem;
    font-weight: 600;
    background: linear-gradient(135deg, #3b82f6 0%, #a855f7 100%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    background-clip: text;
  }

  .nav-links {
    display: flex;
    gap: 1rem;
  }

  .nav-link {
    background: rgba(51, 65, 85, 0.5);
    border: 1px solid rgba(75, 85, 99, 0.3);
    border-radius: 0.5rem;
    padding: 0.5rem 1rem;
    color: white;
    text-decoration: none;
    transition: all 0.2s;
  }

  .nav-link:hover {
    background: rgba(51, 65, 85, 0.8);
  }

  .nav-link.active {
    background: linear-gradient(135deg, #3b82f6 0%, #1d4ed8 100%);
    border-color: #3b82f6;
  }
</style>