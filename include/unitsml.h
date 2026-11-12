#ifndef UNITSML_H
#define UNITSML_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
uint32_t unitsml_abi_version(void);
/* Input is borrowed for the duration of the call. The returned UTF-8
 * NUL-terminated JSON buffer belongs to the caller. */
char *unitsml_evaluate_json_v1(const uint8_t *input, size_t length);
void unitsml_free_json_v1(char *reply);
#ifdef __cplusplus
}
#endif
#endif
