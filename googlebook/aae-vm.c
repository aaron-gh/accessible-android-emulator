/* Runs UTM's QEMU library as a process of its own, named aae-vm, with the
 * hypervisor entitlement QEMU's hvf accelerator needs. The library's path is
 * in AAE_QEMU_LIBRARY. */
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>

int main(int argc, char **argv, char **envp) {
    const char *path = getenv("AAE_QEMU_LIBRARY");
    if (!path) {
        fprintf(stderr, "AAE_QEMU_LIBRARY isn't set\n");
        return 1;
    }
    void *qemu = dlopen(path, RTLD_LOCAL | RTLD_LAZY);
    if (!qemu) {
        fprintf(stderr, "%s\n", dlerror());
        return 1;
    }
    void (*init)(int, char **, char **) = (void (*)(int, char **, char **))dlsym(qemu, "qemu_init");
    void (*loop)(void) = (void (*)(void))dlsym(qemu, "qemu_main_loop");
    void (*cleanup)(void) = (void (*)(void))dlsym(qemu, "qemu_cleanup");
    if (!init || !loop || !cleanup) {
        fprintf(stderr, "%s isn't a QEMU library\n", path);
        return 2;
    }
    init(argc, argv, envp);
    loop();
    cleanup();
    return 0;
}
