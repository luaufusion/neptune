((files, fileorder) => {
    /** @type {{[modname: string]: string}} */
    const internalSources = JSON.parse(files)
    /** @type {string[]} */
    const fileOrder = JSON.parse(fileorder)
    const internalCache = {};
    const primordials = {};

    const internalRequire = (id) => {
        // If its cached, return what it exported
        if (internalCache[id]) return internalCache[id].exports;

        // Load and set to no exports initially
        const source = internalSources[id];
        if (!source) throw new Error(`Native module not found: ${id}`);

        const module = { exports: {} };
        internalCache[id] = module;

        // We need module.expirts, require, our globals and our primordials object for this
        // so we build a wrapper function to bootstrap the runtime
        const wrapper = new Function(
            "exports",
            "require",
            "module",
            "primordials",
            "globalThis",
            `${source}\n//# sourceURL=neptune://internal/${id}`
        );

        wrapper(
            module.exports,
            internalRequire,
            module,
            primordials,
            globalThis
        );

        return module.exports;
    };

    fileOrder.forEach(modId => internalRequire(modId));
})