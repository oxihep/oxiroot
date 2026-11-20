// Dev-only RNTuple fixture generator for the associative containers:
// std::set<int32_t> and std::map<int32_t, double>. ROOT 6.40 writes (and
// reads) a std::map RNTuple only with the map's dictionary compiled in; without
// it, and from the cling interpreter, its collection proxy crashes. So:
//
//   rootcling -f /tmp/rntuple_map_dict.cxx scripts/rntuple_map_LinkDef.h
//   c++ -std=c++20 $(root-config --cflags) -I/opt/homebrew/include -I. \
//       scripts/gen_rntuple_assoc.cpp /tmp/rntuple_map_dict.cxx \
//       $(root-config --libs) -lROOTNTuple -o /tmp/gen_rntuple_assoc
//   /tmp/gen_rntuple_assoc        # run from the repo root
//
// (The extra include path is needed because ROOT's RVec.hxx pulls in vdt, a
// separate Homebrew formula not on ROOT's default include path.)

#include <cstdint>
#include <map>
#include <set>

#include <ROOT/RNTupleModel.hxx>
#include <ROOT/RNTupleWriteOptions.hxx>
#include <ROOT/RNTupleWriter.hxx>

static void write_rntuple(const char *path, int compression) {
    auto model = ROOT::RNTupleModel::Create();
    auto fSet = model->MakeField<std::set<std::int32_t>>("s");

    ROOT::RNTupleWriteOptions options;
    options.SetCompression(compression);
    auto writer = ROOT::RNTupleWriter::Recreate(std::move(model), "ntpl", path, options);

    *fSet = {1, 2, 3};
    writer->Fill();
    *fSet = {4, 5};
    writer->Fill();
    *fSet = {}; // an empty set
    writer->Fill();
    *fSet = {10, 20, 30, 40};
    writer->Fill();
}

static void write_map(const char *path, int compression) {
    auto model = ROOT::RNTupleModel::Create();
    auto fMap = model->MakeField<std::map<std::int32_t, double>>("m");

    ROOT::RNTupleWriteOptions options;
    options.SetCompression(compression);
    auto writer = ROOT::RNTupleWriter::Recreate(std::move(model), "ntpl", path, options);

    *fMap = {{2, 2.5}, {1, 1.5}}; // stored sorted by key
    writer->Fill();
    *fMap = {}; // an empty map
    writer->Fill();
    *fMap = {{3, 3.5}, {-7, 0.25}, {40, -1.0}};
    writer->Fill();
}

int main() {
    write_rntuple("fixtures/rntuple_set_uncompressed.root", 0);
    write_rntuple("fixtures/rntuple_set_zstd.root", 505);
    write_map("fixtures/rntuple_map_uncompressed.root", 0);
    write_map("fixtures/rntuple_map_zstd.root", 505);
    return 0;
}
