const overlay = document.querySelector("#status-overlay");
const label = document.querySelector("#status-label");
const title = document.querySelector("#status-title");
const detail = document.querySelector("#status-detail");

function formatError(error) {
  if (error instanceof Error && error.message) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  try {
    return JSON.stringify(error);
  } catch {
    return "The application stopped before rendering a frame.";
  }
}

function showError(message, error) {
  document.body.dataset.state = "error";
  overlay.hidden = false;
  overlay.setAttribute("role", "alert");
  overlay.setAttribute("aria-live", "assertive");
  label.textContent = "Unable to start";
  title.textContent = message;
  detail.textContent = formatError(error);
}

function waitForFirstFrame(signal) {
  return new Promise((resolve, reject) => {
    const cleanup = () => {
      window.removeEventListener("chad-ready", ready);
      signal.removeEventListener("abort", abort);
      window.clearTimeout(timeout);
    };
    const ready = () => {
      cleanup();
      resolve();
    };
    const abort = () => {
      cleanup();
      resolve();
    };
    const timeout = window.setTimeout(() => {
      cleanup();
      reject(new Error("chad did not present a frame within 30 seconds."));
    }, 30_000);

    window.addEventListener("chad-ready", ready, { once: true });
    signal.addEventListener("abort", abort, { once: true });
  });
}

export async function launch(loadApplication) {
  let settled = false;

  const startupError = (event) => {
    if (!settled) {
      settled = true;
      showError("The WebGPU application failed during startup.", event.error ?? event.message);
    }
  };
  const startupRejection = (event) => {
    if (!settled) {
      settled = true;
      showError("The WebGPU application failed during startup.", event.reason);
    }
  };

  window.addEventListener("error", startupError);
  window.addEventListener("unhandledrejection", startupRejection);

  if (!navigator.gpu) {
    settled = true;
    showError(
      "WebGPU is not available in this browser.",
      "Use a current WebGPU-capable browser and ensure hardware acceleration is enabled.",
    );
    return;
  }

  const startup = new AbortController();
  try {
    const application = await loadApplication();
    if (typeof application.default !== "function") {
      throw new TypeError("pkg/app.js does not export a default initializer.");
    }

    const firstFrame = waitForFirstFrame(startup.signal);
    await application.default();
    await firstFrame;

    settled = true;
    window.removeEventListener("error", startupError);
    window.removeEventListener("unhandledrejection", startupRejection);
    document.body.classList.add("is-ready");
    delete document.body.dataset.state;
    overlay.hidden = true;
    startup.abort();
  } catch (error) {
    settled = true;
    startup.abort();
    window.removeEventListener("error", startupError);
    window.removeEventListener("unhandledrejection", startupRejection);
    showError("The WebGPU application could not be initialized.", error);
  }
}
