/* Adds allow rules to a compiled SELinux policy.
 *   sepolicy-allow IN OUT source:target:class:perm,perm ...
 * Prints each rule with the permission bits it added. Changes nothing else:
 * not permissive domains, capabilities, MLS or constraints. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sepol/policydb/policydb.h>
#include <sepol/policydb/hashtab.h>
#include <sepol/policydb/avtab.h>

int main(int argc, char **argv) {
    if (argc < 3) {
        fprintf(stderr, "usage: %s IN OUT source:target:class:perm,perm ...\n", argv[0]);
        return 2;
    }
    policydb_t p;
    policy_file_t f;
    policydb_init(&p);
    policy_file_init(&f);
    f.type = PF_USE_STDIO;
    f.fp = fopen(argv[1], "rb");
    if (!f.fp || policydb_read(&p, &f, 0)) {
        fprintf(stderr, "can't read %s\n", argv[1]);
        return 3;
    }
    fclose(f.fp);
    for (int i = 3; i < argc; i++) {
        char rule[512];
        if (strlen(argv[i]) >= sizeof rule) return 4;
        strcpy(rule, argv[i]);
        char *src = strtok(rule, ":"), *tgt = strtok(NULL, ":"), *cls = strtok(NULL, ":"), *list = strtok(NULL, ":");
        if (!src || !tgt || !cls || !list) {
            fprintf(stderr, "bad rule %s\n", argv[i]);
            return 4;
        }
        type_datum_t *s = hashtab_search(p.p_types.table, src), *t = hashtab_search(p.p_types.table, tgt);
        class_datum_t *k = hashtab_search(p.p_classes.table, cls);
        if (!s || !t || !k) {
            fprintf(stderr, "unknown type or class in %s\n", argv[i]);
            return 5;
        }
        unsigned bits = 0;
        for (char *perm = strtok(list, ","); perm; perm = strtok(NULL, ",")) {
            perm_datum_t *v = hashtab_search(k->permissions.table, perm);
            if (!v && k->comdatum) v = hashtab_search(k->comdatum->permissions.table, perm);
            if (!v || !v->s.value || v->s.value > 32) {
                fprintf(stderr, "unknown permission %s in %s\n", perm, argv[i]);
                return 6;
            }
            bits |= 1U << (v->s.value - 1);
        }
        avtab_key_t key = {.source_type = s->s.value, .target_type = t->s.value,
                           .target_class = k->s.value, .specified = AVTAB_ALLOWED};
        avtab_datum_t *old = avtab_search(&p.te_avtab, &key);
        printf("allow %s: had %08x, added %08x\n", argv[i], old ? old->data : 0, bits & ~(old ? old->data : 0));
        if (old) {
            old->data |= bits;
        } else {
            avtab_datum_t value = {.data = bits};
            if (avtab_insert(&p.te_avtab, &key, &value)) return 7;
        }
    }
    f.fp = fopen(argv[2], "wb");
    if (!f.fp || policydb_write(&p, &f)) {
        fprintf(stderr, "can't write %s\n", argv[2]);
        return 8;
    }
    fclose(f.fp);
    policydb_destroy(&p);
    return 0;
}
