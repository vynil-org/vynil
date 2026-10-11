// Décode un texte multi-documents en ne gardant que les cartes utilisables.
// vynil-core 0.8.0 : un document vide devient `()` et les documents courts ne sont plus filtrés.
fn decode(text) {
    let docs = [];
    for d in yaml_decode_multi(text) {
        if type_of(d) == "map" {
            docs.push(d);
        } else {
            log_debug(`yaml_docs: document écarté (type ${type_of(d)})`);
        }
    }
    docs
}
