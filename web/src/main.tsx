import { render } from "preact";
import { App } from "./app";
import { applyTheme, loadTheme } from "./theme";
import "./styles.css";
import "./theme.css";

applyTheme(loadTheme());

render(<App />, document.getElementById("app")!);
