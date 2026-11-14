#include "network_shim.h"

#include <QtNetwork/QNetworkInformation>

// No backend loaded yet, or loading failed. A single slot is enough: the
// function is called from one worker thread at a time and only ever asks the
// same question.
namespace {

enum : int { NOT_LOADED = -1, UNKNOWN = 0 };

int g_backend = NOT_LOADED;
bool g_loaded = false;

} // namespace

int kxbps_network_reachability()
{
    // `loadDefaultBackend` is documented to return true when a backend is
    // already loaded, so a repeated call is cheap; the guard keeps the failure
    // path from retrying a load that cannot succeed.
    if (!g_loaded) {
        g_loaded = true;
        // On Linux the platform backend is NetworkManager. Without it Qt 6.7+
        // falls back to a backend that only reports Unknown, which the caller
        // reads as "try the network".
        if (!QNetworkInformation::loadDefaultBackend()) {
            g_backend = NOT_LOADED;
            return g_backend;
        }
    }
    const QNetworkInformation *info = QNetworkInformation::instance();
    if (info == nullptr) {
        g_backend = UNKNOWN;
        return g_backend;
    }
    g_backend = static_cast<int>(info->reachability());
    return g_backend;
}
