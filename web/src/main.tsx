import { render } from "preact";
import { App } from "./app";
import { FileBrowserProvider } from "./components/FileBrowser";
import { applyTheme, loadTheme } from "./theme";
import "./styles.css";
import "./theme.css";
import "./minimal.css";

applyTheme(loadTheme());

render(
  <FileBrowserProvider>
    <App />
  </FileBrowserProvider>,
  document.getElementById("app")!,
);
