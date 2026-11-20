// Dictionary for the std::map<int32_t, double> RNTuple field, used by the
// fixture generator (scripts/gen_rntuple_assoc.cpp) and the interop matrix's
// ROOT oracle: ROOT 6.40 needs it to write or read such a field, even from
// compiled code.
#include <cstdint>
#include <map>
#include <utility>

#ifdef __ROOTCLING__
#pragma link C++ class std::pair<std::int32_t, double>+;
#pragma link C++ class std::map<std::int32_t, double>+;
#endif
