// Dev-only RNTuple fixture generator: a std::variant field over three
// clusters. A switch entry's index counts its alternative's values within the
// cluster, so a reader must add the values of the earlier clusters.
//
//   c++ $(root-config --cflags) scripts/gen_rntuple_variant_clusters.cpp \
//       $(root-config --libs) -lROOTNTuple -o /tmp/gen_rntuple_variant_clusters
//   /tmp/gen_rntuple_variant_clusters        # run from the repo root

#include <cstdint>
#include <variant>

#include <ROOT/RNTupleModel.hxx>
#include <ROOT/RNTupleWriteOptions.hxx>
#include <ROOT/RNTupleWriter.hxx>

int main() {
    auto model = ROOT::RNTupleModel::Create();
    auto v = model->MakeField<std::variant<std::int32_t, double>>("v");
    ROOT::RNTupleWriteOptions options;
    options.SetCompression(0);
    auto writer = ROOT::RNTupleWriter::Recreate(std::move(model), "ntpl",
                                                "fixtures/rntuple_variant_clusters.root", options);
    // Entry i: an int i when i % 3 == 0, else a double i + 0.5; clusters of
    // 100, 250 and 150 entries.
    for (int i = 0; i < 500; ++i) {
        if (i % 3 == 0)
            *v = std::int32_t(i);
        else
            *v = i + 0.5;
        writer->Fill();
        if (i == 99 || i == 349)
            writer->CommitCluster();
    }
    return 0;
}
