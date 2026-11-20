// Dev-only RNTuple fixture generator: collection and string offsets spread
// over many pages per cluster. ROOT delta-encodes a split index column page by
// page, so a reader must restart the sum at every page boundary.
//
//   c++ $(root-config --cflags) scripts/gen_rntuple_pages.cpp \
//       $(root-config --libs) -lROOTNTuple -o /tmp/gen_rntuple_pages
//   /tmp/gen_rntuple_pages        # run from the repo root

#include <string>
#include <vector>

#include <ROOT/RNTupleModel.hxx>
#include <ROOT/RNTupleWriteOptions.hxx>
#include <ROOT/RNTupleWriter.hxx>

int main() {
    auto model = ROOT::RNTupleModel::Create();
    auto jets = model->MakeField<std::vector<double>>("jets");
    auto label = model->MakeField<std::string>("label");

    ROOT::RNTupleWriteOptions options;
    options.SetCompression(505);
    // Small pages: the 8-byte offsets of 2500 entries fill about 20 of them.
    options.SetInitialUnzippedPageSize(64);
    options.SetMaxUnzippedPageSize(1024);
    auto writer = ROOT::RNTupleWriter::Recreate(std::move(model), "ntpl",
                                                "fixtures/rntuple_multipage.root", options);
    // Entry i: jets = i % 5 values (i * 5 + j) * 0.5, label = "e" + i;
    // two clusters of 2500 entries.
    for (int i = 0; i < 5000; ++i) {
        jets->clear();
        for (int j = 0; j < i % 5; ++j)
            jets->push_back((i * 5 + j) * 0.5);
        *label = "e" + std::to_string(i);
        writer->Fill();
        if (i == 2499)
            writer->CommitCluster();
    }
    return 0;
}
