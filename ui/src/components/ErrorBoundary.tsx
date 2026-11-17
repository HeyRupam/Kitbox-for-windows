import { Component, type ErrorInfo, type ReactNode } from "react";

import { Icon, ICONS } from "./Icon";

/**
 * Catches a render error so one broken screen can't take the window with it.
 *
 * React's rule is blunt: an error thrown during render unmounts the whole
 * tree. Without a boundary that means a white window and no clue — which is
 * exactly what a mismatched field name in one results list produced once, and
 * the app looked dead rather than wrong.
 *
 * `resetKey` is normally the current screen. When it changes the boundary
 * clears itself, so leaving a broken screen is enough to get back to work.
 */
export class ErrorBoundary extends Component<
  {
    children: ReactNode;
    resetKey?: string;
    /** What broke, in the user's terms: "Compress image", "Kitbox". */
    what: string;
    /** Set when nothing around this supplies the window's padding. */
    inset?: boolean;
    onBack?: () => void;
  },
  { error: Error | null; where: string }
> {
  state: { error: Error | null; where: string } = { error: null, where: "" };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    // The component stack says which part of the screen threw, which the
    // message alone never does — but only in a development build. A release
    // bundle is minified, so those frames read "at i" and take up room saying
    // nothing. The console still gets the whole thing either way.
    const frames = (info.componentStack ?? "")
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean)
      .slice(0, 4);
    const legible = frames.filter((frame) => /at [A-Za-z][\w.]{2,}/.test(frame));
    this.setState({ where: legible.join("  ·  ") });
    console.error("Kitbox: a screen stopped", error, info.componentStack);
  }

  componentDidUpdate(previous: { resetKey?: string }) {
    if (this.state.error && previous.resetKey !== this.props.resetKey) {
      this.setState({ error: null, where: "" });
    }
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;

    return (
      <div
        className="col grow"
        style={{ gap: 22, minHeight: 0, padding: this.props.inset ? 22 : 0 }}
      >
        <section
          className="raised col"
          style={{ borderRadius: "var(--r-panel)", padding: 28, gap: 16, maxWidth: 680 }}
        >
          <span className="row" style={{ gap: 10, alignItems: "center" }}>
            <Icon path={ICONS.warning} size={22} strokeWidth={2} />
            <h1 className="display" style={{ fontSize: 22 }}>
              {this.props.what} stopped
            </h1>
          </span>

          <p style={{ color: "var(--text-muted)", lineHeight: 1.7, margin: 0 }}>
            Nothing you were working on was sent anywhere, and nothing already saved is
            affected. Your files are untouched — this went wrong while drawing the screen,
            not while working on them.
          </p>

          <div className="pressed col scroll" style={{ borderRadius: 16, padding: 14, gap: 6 }}>
            <span style={{ fontSize: 12.5, fontWeight: 600 }}>{error.message}</span>
            {this.state.where && (
              <span style={{ fontSize: 11.5, color: "var(--text-subtle)", lineHeight: 1.6 }}>
                {this.state.where}
              </span>
            )}
          </div>

          <div className="row" style={{ gap: 10 }}>
            <button
              type="button"
              className="btn btn-primary"
              style={{ minHeight: 46, borderRadius: 14 }}
              onClick={() => this.setState({ error: null, where: "" })}
            >
              Try this screen again
            </button>
            {this.props.onBack && (
              <button
                type="button"
                className="raised-sm btn"
                style={{ minHeight: 46 }}
                onClick={() => {
                  this.setState({ error: null, where: "" });
                  this.props.onBack?.();
                }}
              >
                <Icon path={ICONS.back} size={18} strokeWidth={2} />
                Back to all tools
              </button>
            )}
            <button
              type="button"
              className="raised-sm btn"
              style={{ minHeight: 46 }}
              onClick={() => {
                void navigator.clipboard?.writeText(
                  `${error.message}\n\n${error.stack ?? ""}\n\n${this.state.where}`,
                );
              }}
            >
              Copy the details
            </button>
          </div>
        </section>
      </div>
    );
  }
}
