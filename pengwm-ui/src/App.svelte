<script lang="ts">
  import { PaneGroup, Pane, PaneResizer } from "paneforge";
  import { Layout, Settings, Keyboard, Monitor } from "lucide-svelte";

  let name = $state('');
  let greeting = $state('');
  let maxTiles = $state(4);

  async function greet() {
    greeting = `Hello, ${name}! (Mocked greeting)`;
  }
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
        <label>Max Tiles:</label>
        <input type="number" bind:value={maxTiles} min="1" max="10" />
      </div>
    </header>

    <div class="layout-preview">
      <PaneGroup direction="horizontal">
        <Pane defaultSize={50} class="tile">
          <div class="tile-content">Window 1</div>
        </Pane>
        <PaneResizer class="resizer" />
        <Pane defaultSize={50}>
          <PaneGroup direction="vertical">
            <Pane defaultSize={50} class="tile">
              <div class="tile-content">Window 2</div>
            </Pane>
            <PaneResizer class="resizer" />
            <Pane defaultSize={50} class="tile">
              <div class="tile-content">Window 3</div>
            </Pane>
          </PaneGroup>
        </Pane>
      </PaneGroup>
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
    overflow: hidden;
    border: 1px solid #444;
  }

  :global(.tile) {
    background-color: #2a2a2a;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .tile-content {
    font-size: 1.2rem;
    font-weight: bold;
    color: #888;
  }

  :global(.resizer) {
    width: 4px;
    background-color: #444;
    transition: background-color 0.2s;
  }

  :global(.resizer:hover) {
    background-color: #ff3e00;
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
