/* @refresh reload */
import "@tailwindcss/vite";
import { render } from "solid-js/web";
import LiteApp from "./LiteApp";

render(() => <LiteApp />, document.getElementById("root")!);
