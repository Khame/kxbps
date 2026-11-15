.pragma library

// Decoding the JSON the backend hands QML.
//
// The backend publishes lists as JSON arrays in a string property, because a
// `QList` of structs does not cross the bridge. Every reader wants the same
// thing — an array or nothing — so the fallback lives here instead of in a
// `try` block per view.

/// `json` as an array; `[]` when it is not one.
///
/// A missing property, an empty string and a payload this version does not
/// understand all mean "no rows": the view shows its empty state instead of
/// throwing while it builds.
function array(json) {
    try {
        const parsed = JSON.parse(json)
        return Array.isArray(parsed) ? parsed : []
    } catch (error) {
        return []
    }
}
