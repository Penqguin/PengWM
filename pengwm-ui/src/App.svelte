<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { Layout, Settings, Keyboard, Monitor } from "lucide-svelte";

  interface WindowInfo {
    id: number;
    title: string;
    x: number;
    y: number;
    width: number;
    height: number;
  }

  interface UiState {
    windows: WindowInfo[];
    focused_window: number | null;
  }

  interface UiEvent {
    type: "StateChanged";
    data: UiState;
  }

  let windows = $state<WindowInfo[]>([]);
  let maxTiles = $state(4);

  onMount(() => {
    const unlisten = listen<UiEvent>("state-changed", (event) => {
      console.log("Received state update:", event.payload);
      if (event.payload.type === "StateChanged") {
        windows = event.payload.data.windows;
      }
    });

    return () => {
      unlisten.then(f => f());
    };
  });
</script>

<main class="container">
  <nav class="sidebar">
    <div class="logo">
      <Monitor size={32} />
      <span>pengwm</span>
    </div>
    <ul>
      <li class="active"><Layout size={20} /> Layout</li>
      <li><Keyboard size={20} /> Keybinds</li>
      <li><Settings size={20} /> Settings</li>
    </ul>
  </nav>

  <section class="content">
    <header>
      <h1>Visual Layout Designer</h1>
      <div class="controls">
        <label>
          Max Tiles:
          <input type="number" bind:value={maxTiles} min="1" max="10" />
        </label>
      </div>
    </header>

    <div class="layout-preview">
      {#if windows.length === 0}
        <div class="empty-state">
          No windows managed. Open some apps!
        </div>
      {:else}
        <div class="window-grid">
          {#each windows as win}
            <div class="window-tile" style="
              left: {win.x / 10}px;
              top: {win.y / 10}px;
              width: {win.width / 10}px;
              height: {win.height / 10}px;
            ">
              <span class="window-label">{win.title}</span>
              <span class="window-id">#{win.id}</span>
            </div>
          {/each}
        </div>
      {/if}
    </div>
  </section>
</main>

<style>
  :global(body) {
    margin: 0;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
    background-color: #1a1a1a;
    color: #eee;
  }

  .container {
    display: flex;
    height: 100vh;
  }

  .sidebar {
    width: 200px;
    background-color: #111;
    padding: 1rem;
    border-right: 1px solid #333;
  }

  .logo {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 1.5rem;
    font-weight: bold;
    margin-bottom: 2rem;
  }

  ul {
    list-style: none;
    padding: 0;
  }

  li {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.75rem;
    border-radius: 4px;
    cursor: pointer;
    margin-bottom: 0.25rem;
  }

  li:hover {
    background-color: #222;
  }

  li.active {
    background-color: #333;
    color: #ff3e00;
  }

  .content {
    flex: 1;
    display: flex;
    flex-direction: column;
    padding: 2rem;
  }

  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 2rem;
  }

  .layout-preview {
    flex: 1;
    background-color: #222;
    border-radius: 8px;
    position: relative;
    overflow: hidden;
    border: 1px solid #444;
  }

  .empty-state {
    display: flex;
    height: 100%;
    align-items: center;
    justify-content: center;
    color: #666;
    font-style: italic;
  }

  .window-grid {
    position: relative;
    width: 100%;
    height: 100%;
  }

  .window-tile {
    position: absolute;
    background-color: #2a2a2a;
    border: 1px solid #ff3e00;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    border-radius: 4px;
    transition: all 0.2s ease-out;
  }

  .window-label {
    font-size: 0.9rem;
    font-weight: bold;
    color: #eee;
    text-align: center;
    padding: 4px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 90%;
  }

  .window-id {
    font-size: 0.7rem;
    color: #888;
  }

  .controls {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  input[type="number"] {
    width: 50px;
    background: #333;
    border: 1px solid #444;
    color: white;
    padding: 4px;
    border-radius: 4px;
  }
</style>
