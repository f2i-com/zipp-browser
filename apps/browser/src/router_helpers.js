// Inlined React Router helpers for matchRoutes (v function).
// These are injected at the start of matchRoutes to avoid IIFE-level
// single-letter variable collisions (D, T, O, R, M, F all collide
// with identically-named functions in other webpack-concatenated modules).

// D → __RD (stripBasename)
var __RD = function(e, t) {
    if ("/" === t) return e;
    if (!e.toLowerCase().startsWith(t.toLowerCase())) return null;
    var n = t.endsWith("/") ? t.length - 1 : t.length;
    var r = e.charAt(n);
    return r && "/" !== r ? null : e.slice(n) || "/";
};

// R → __RR (decodePath)
var __RR = function(e) {
    try {
        return e.split("/").map(function(s) {
            return decodeURIComponent(s).replace(/\//g, "%2F");
        }).join("/");
    } catch(t) { return e; }
};

// M → __RM (joinPaths)
var __RM = function(e) {
    return e.join("/").replace(/\/\/+/g, "/");
};

// F → __RF (normalizePathname)
var __RF = function(e) {
    return e.replace(/\/+$/, "").replace(/^\/*/, "/");
};

// O → __RO (matchPath - compilePath + match)
var __RO = function(e, t) {
    if ("string" === typeof e) e = {path: e, caseSensitive: false, end: true};
    var paramNames = [];
    var step1 = e.path.replace(/\/*\*?$/, "");
    var step2 = step1.replace(/^\/*/, "/");
    // Escape regex special chars manually (avoid complex character class regex
    // which causes the Zipp regex engine to hang)
    var step3 = "";
    for (var ci = 0; ci < step2.length; ci++) {
        var ch = step2.charAt(ci);
        if ("\\.*+^${}|()[]".indexOf(ch) >= 0) {
            step3 += "\\" + ch;
        } else {
            step3 += ch;
        }
    }
    var step4 = step3.replace(/\/:([\w-]+)(\?)?/g, function(m, pn, opt) {
        paramNames.push({paramName: pn, isOptional: null != opt});
        return opt ? "/?([^\\/]+)?" : "/([^\\/]+)";
    });
    var src = "^" + step4;
    if (e.path.endsWith("*")) {
        paramNames.push({paramName: "*"});
        src += ("*" === e.path || "/*" === e.path) ? "(.*)$" : "(?:\\/(.+)|\\/*)$";
    } else if (e.end) {
        src += "\\/*$";
    } else if ("" !== e.path && "/" !== e.path) {
        src += "(?:(?=\\/|$))";
    }
    var re = new RegExp(src, e.caseSensitive ? void 0 : "i");
    var m = t.match(re);
    if (!m) return null;
    var matched = m[0];
    var base = matched.replace(/(.)\/+$/, "$1");
    var captures = m.slice(1);
    var params = {};
    for (var pi = 0; pi < paramNames.length; pi++) {
        var pn = paramNames[pi].paramName;
        var isOpt = paramNames[pi].isOptional;
        if ("*" === pn) {
            var star = captures[pi] || "";
            base = matched.slice(0, matched.length - star.length).replace(/(.)\/+$/, "$1");
        }
        var cv = captures[pi];
        params[pn] = isOpt && !cv ? void 0 : (cv || "").replace(/%2F/g, "/");
    }
    return {params: params, pathname: matched, pathnameBase: base, pattern: e};
};

// T → __RT (matchRouteBranch)
var __RT = function(branch, pathname, allowPartial) {
    if (void 0 === allowPartial) allowPartial = false;
    var metas = branch.routesMeta;
    var params = {};
    var base = "/";
    var matches = [];
    for (var idx = 0; idx < metas.length; ++idx) {
        var meta = metas[idx];
        var isLast = idx === metas.length - 1;
        var remaining = "/" === base ? pathname : pathname.slice(base.length) || "/";
        var match = __RO(
            {path: meta.relativePath, caseSensitive: meta.caseSensitive, end: isLast},
            remaining
        );
        var route = meta.route;
        if (!match && isLast && allowPartial && !metas[metas.length - 1].route.index) {
            match = __RO(
                {path: meta.relativePath, caseSensitive: meta.caseSensitive, end: false},
                remaining
            );
        }
        if (!match) return null;
        Object.assign(params, match.params);
        matches.push({
            params: params,
            pathname: __RM([base, match.pathname]),
            pathnameBase: __RF(__RM([base, match.pathnameBase])),
            route: route
        });
        if ("/" !== match.pathnameBase) base = __RM([base, match.pathnameBase]);
    }
    return matches;
};
